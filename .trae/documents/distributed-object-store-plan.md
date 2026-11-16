# 分布式对象存储（raw_store + 分布式 S3 网关）实施方案

## Context

现有对象存储 `laoflchdb_object_store_service` 是单机形态：S3 协议层（`src/s3.rs`）与
`ObjectStoreServiceImpl` 紧耦合，数据全部落在本机 RocksDB/BlobDB，无法横向扩展。

本次新增一套**完全独立、与现有 object_store 并存**的分布式对象存储能力：

- **raw_store**：数据面节点，通过 gRPC 接收 chunk，由新引擎 **raw_storage_engine**
  直接写入本地文件系统中的 chunk 文件；
- **分布式 S3 网关**：每个网关节点都提供完整 S3 接入；元数据写现有 Raft 集群；对象按
  **4MB 分块、3 副本、一致性哈希**放置到所有 raw_store 节点；任意 S3 网关都能访问全部
  raw_store 节点。

**硬约束：现有 `laoflchdb_object_store_service`（含 `lib.rs`、`s3.rs`）以及 Raft 服务代码
一律不修改。** 新网关自带一套 S3 协议实现（从现有实现适配复制 SigV4 / BucketPolicy /
Range / List 等成熟逻辑），仅新增 crate 与主程序的加法式接线。

## 总体架构

```
S3 client
   │
   ▼
┌──────────────────────────┐   元数据 Put/Get/Delete/Scan（现有 Raft gRPC）
│ 分布式 S3 网关 (N 个)     │ ───────────────────────────────► Raft 集群
│  - 自带 S3 协议层         │                                  objmeta 组（对象元数据）
│  - 分块 / 一致性哈希      │                                  raw_registry 组（节点注册）
│  - Raft 元数据客户端      │
└──────────────────────────┘   chunk PutChunk/GetChunk/DeleteChunk（gRPC）
              │ ───────────────────────────────────────────────► raw_store 1..M
                                                              │
                                                              ▼
                                                   raw_storage_engine
                                                   {root}/ab/cd/{chunk_id}.chunk
```

- **部署形态**：同一 `laoflchdb` 二进制 + 不同 yaml（与现有 raft-node 模式一致），不新增 bin target。
- **节点发现**：静态种子列表 + Raft 注册表（`raw_registry` 组），网关周期刷新并按 TTL 过滤。
- **放置**：一致性哈希环（每节点 128 个虚拟节点），顺时针取 3 个不同节点；写 quorum = 2。
- **默认端口**：raw_store gRPC `29998`，分布式 S3 `19001`（均可配置）。
- **复用现有 Raft**：只调用其既有 `Put/Get/Delete/Scan` 接口，**不修改 Raft proto 与服务端**。
  建议在 `raft_service.groups` 中新增两个独立组：`objmeta`、`raw_registry`。

## 新增 / 修改清单

### 1. 新 crate `laoflchdb_raw_storage_engine`

文件：`laoflchdb_raw_storage_engine/{Cargo.toml,src/lib.rs}`

- 在本 crate 内定义 chunk 存储抽象（不改 `laoflchdb_engines`）：

```rust
#[async_trait::async_trait]
pub trait ChunkStorage: Send + Sync + 'static {
    fn engine_type(&self) -> &str;
    async fn put_chunk(&self, chunk_id: &str, data: &[u8]) -> Result<(), Box<dyn Error + Send + Sync>>;
    async fn get_chunk(&self, chunk_id: &str) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>>;
    async fn delete_chunk(&self, chunk_id: &str) -> Result<(), Box<dyn Error + Send + Sync>>;
    async fn stat_chunk(&self, chunk_id: &str) -> Result<Option<ChunkStat>, Box<dyn Error + Send + Sync>>;
    async fn health(&self) -> Result<(), Box<dyn Error + Send + Sync>>;
}
pub struct ChunkStat { pub size: u64, pub sha256: [u8; 32] }
```

- `RawStorageConfig { root: String, fsync: bool, shard_depth: usize }`（默认 shard_depth = 2）。
- 文件布局：`{root}/{chunk_id[0..2]}/{chunk_id[2..4]}/{chunk_id}.chunk`。
- 文件格式：`[magic "LCHK" 4B][sha256 32B][data_len u64 LE][data]`，`stat_chunk` 只读头部。
- 写入：临时文件 + `create_dir_all` + `sync_all`（可配置）+ 原子 `rename`；删除幂等；
  `get_chunk` 校验 sha256。
- **不修改** `laoflchdb_engines`、`laoflchdb_kv_rocksdb_engine`。

### 2. 新 crate `laoflchdb_raw_store_service`

文件：`laoflchdb_raw_store_service/{Cargo.toml,build.rs,proto/raw_store.proto,src/lib.rs,src/client.rs,src/registry.rs}`

proto（`package laoflchdb.raw_store`）：

```proto
service RawStoreService {
  rpc PutChunk(PutChunkRequest) returns (PutChunkResponse);
  rpc GetChunk(GetChunkRequest) returns (GetChunkResponse);
  rpc DeleteChunk(DeleteChunkRequest) returns (DeleteChunkResponse);
  rpc StatChunk(StatChunkRequest) returns (StatChunkResponse);
  rpc Health(HealthRequest) returns (HealthResponse);
}
```

- `PutChunkRequest { string chunk_id; bytes data; }`（chunk 4MB，单帧可容纳）。
- `RawStoreServiceImpl { engine: Arc<dyn ChunkStorage>, node_id, advertise_addr }`。
- `client.rs`：`RawStoreClient` / `RawStoreClientPool`（tonic channel 复用）。
- `registry.rs`：后台任务周期（默认 10s）通过现有 Raft gRPC `Put` 写
  `r\0{node_id}` = JSON `{node_id, addr, updated_at}` 到 `raw_registry` 组。
- `start_raw_store_service(service)`：独立 gRPC server，`max_frame_size = 16MB`。
- 依赖：`laoflchdb_raw_storage_engine`、`laoflchdb_raft_service`（仅复用其公开的 `proto` 客户端）。

### 3. 新 crate `laoflchdb_distributed_object_store`

文件：`laoflchdb_distributed_object_store/{Cargo.toml,src/lib.rs,src/config.rs,src/s3_protocol.rs,src/raft_meta.rs,src/registry.rs,src/placement.rs,src/chunk_io.rs,src/backend.rs,src/multipart.rs}`

- **`s3_protocol.rs`**：本 crate 自带的流式 S3 协议层。从现有 `s3.rs` 适配复制
  SigV4 校验、BucketPolicy 匿名读、List V1/V2、Delimiter、Range 解析等逻辑；
  现有 `s3.rs` 文件保持不动。协议层直接对接后端 trait（见下），不依赖旧 object_store 服务。
- **后端抽象**（本 crate 内定义）：

```rust
#[async_trait]
pub trait S3Backend: Send + Sync + 'static {
    async fn list_buckets(&self) -> Result<Vec<S3BucketInfo>, S3Error>;
    async fn create_bucket(&self, bucket: &str) -> Result<(), S3Error>;
    async fn delete_bucket(&self, bucket: &str) -> Result<(), S3Error>;
    async fn get_bucket_policy(&self, bucket: &str) -> Result<Option<String>, S3Error>;
    async fn put_bucket_policy(&self, bucket: &str, policy: &str) -> Result<(), S3Error>;
    async fn delete_bucket_policy(&self, bucket: &str) -> Result<(), S3Error>;
    async fn list_objects(&self, bucket: &str, q: S3ListQuery) -> Result<S3ListResult, S3Error>;
    async fn put_object(&self, bucket: &str, key: &str, content_type: &str,
                        metadata: HashMap<String, String>, body: axum::body::Body) -> Result<S3PutResult, S3Error>;
    async fn get_object(&self, bucket: &str, key: &str, range: Option<(u64, u64)>) -> Result<S3GetResult, S3Error>;
    async fn head_object(&self, bucket: &str, key: &str) -> Result<S3HeadResult, S3Error>;
    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), S3Error>;
    async fn delete_objects(&self, bucket: &str, keys: &[String]) -> Result<Vec<String>, S3Error>;
    async fn copy_object(&self, src: (&str, &str), dst: (&str, &str)) -> Result<String, S3Error>;
    // multipart：Create/UploadPart/Complete/Abort
}
```

- **流式处理**：
  - 签名校验优先使用 `x-amz-content-sha256` 头（无需预读 body）；缺失时回退缓冲（带上限）；
  - `PutObject` / `UploadPart` 直接把 `axum::body::Body` 交给 backend；
  - `GetObject` 由 backend 返回 `Body`，协议层只设置状态码 / Content-Range / ETag 等头。
- **`config.rs`**：

```yaml
distributed_object_store:
  enabled: true
  listen_addr: "0.0.0.0:19001"
  s3_access_key: admin
  s3_secret_key: laoflchdb
  s3_region: us-east-1
  chunk_size_bytes: 4194304       # 4MB
  replication_factor: 3
  write_quorum: 2
  raw_store_seeds:                # 静态种子
    - "raw-store-1:29998"
    - "raw-store-2:29998"
    - "raw-store-3:29998"
  registry_ttl_secs: 60
  registry_refresh_secs: 10
  raft_meta:
    endpoints: ["raft-node-1:29999", "raft-node-2:29999", "raft-node-3:29999"]
    group_id: "objmeta"            # 对象/桶元数据
    registry_group_id: "raw_registry"  # raw_store 节点注册
```

- **`raft_meta.rs`**：`RaftMetaClient`，复用 `laoflchdb_raft_service::proto` 客户端，
  提供 `put/get/delete/scan`，endpoint 轮询/失败切换；写请求由 Raft 服务自动转发 leader。
- **`registry.rs`**：`RawStoreRegistry`
  - 合并静态种子与 `raw_registry` 组 `r\0` 前缀项，按 `updated_at` + TTL 过滤；
  - 周期刷新 + `Health` 探测，剔除不可用节点；
  - 维护一致性哈希环（每节点 128 vnode，`hash(node_id#i)`）。
- **`placement.rs`**：`replicas_for(chunk_id) -> Vec<u64>`，顺时针取 `replication_factor`
  个不同节点；节点不足时降级为可用节点数。
- **`chunk_io.rs`**：
  - 写：流式读 body → 4MB 缓冲 → 生成 `chunk_id`（32 hex 随机）+ sha256 →
    并发写 3 副本，成功数 ≥ `write_quorum` 才继续；失败回滚已写 chunk；
  - 读：顺序遍历 chunk，副本按元数据顺序 failover，校验 sha256；
    仅 Range 重叠的 chunk 被读取，并对首尾 chunk 裁剪；输出 `Body::from_stream`。
- **`backend.rs`**：`DistributedObjectStore` 实现 `S3Backend`。
- **`multipart.rs`**：Create / UploadPart / Complete / Abort；Complete 将 part 的 chunk 列表
  拼接为对象 chunk 列表并写 `o\0` 元数据。
- **`lib.rs`**：`start_distributed_object_store(config)` 构建 registry + backend +
  S3 router，绑定 `listen_addr` 启动 axum。

元数据键（NUL 分隔）：

| 组 | 键 | 值 |
| --- | --- | --- |
| objmeta | `b\0{bucket}` | `{name, creation_date, policy}` |
| objmeta | `o\0{bucket}\0{key}` | `{key,size,etag,content_type,last_modified,user_metadata,chunks:[{id,offset,size,sha256,replicas}]}` |
| objmeta | `u\0{bucket}\0{upload_id}` | `{upload_id,bucket,key,content_type,user_metadata,created_at}` |
| objmeta | `p\0{bucket}\0{upload_id}\0{part:05}` | `{part_number,size,etag,chunks:[...]}` |
| raw_registry | `r\0{node_id}` | `{node_id,addr,updated_at}` |

- 由于不改 Raft 的前缀扫描接口，`ListObjects` 对 `objmeta` 组执行全量 `Scan` 后本地过滤；
  注册表单独放在 `raw_registry` 小分组，避免每次刷新扫描对象元数据。规模增大后可再作为
  后续优化引入前缀扫描（不阻塞本期）。
- 一致性说明：写入经 Raft 提交；读取直查配置的 Raft endpoint 的状态机，存在极小的
  follower apply 滞后，属最终一致。

### 4. 主程序与配置接线（仅加法，不影响现有 object_store）

- `src/config/mod.rs`：`DatabaseConfig` 增加两个可选字段
  `raw_store: Option<RawStoreConfig>`、`distributed_object_store: Option<DistributedObjectStoreConfig>`
  （类型来自新 crate）。
- `src/server/mod.rs`：**仅新增分支**，现有 object_store / s3 分支保持原样：
  - `raw_store.enabled` 时：创建 `RawStorageEngine` + `RawStoreServiceImpl`，
    spawn 独立 gRPC server；配置了 raft 时 spawn 注册心跳任务；
  - `distributed_object_store.enabled` 时：构建 `DistributedObjectStore`，
    创建 S3 router 并绑定 `listen_addr` 启动。
- `Cargo.toml`（workspace）：加入 3 个新 crate 成员及根依赖。
- `config/prod.yaml`：新增两段**默认 `enabled: false`** 的注释示例，不改变现有运行行为。

**明确不改动的文件**：`laoflchdb_object_store_service/src/lib.rs`、`src/s3.rs`、
`laoflchdb_raft_service/*`、`laoflchdb_engines/*`、`laoflchdb_kv_rocksdb_engine/*`。

## 关键实现细节

- `chunk_id`：32 位十六进制随机串；每 chunk 计算 sha256 并在写入/读取时校验。
- **PutObject**：流式分块 → 每块并发写 3 副本、quorum 2 → 全部成功后一次性写对象元数据
  （提交点）；元数据写失败则回滚已写 chunk。
- **GetObject**：读元数据 → 顺序流式拉取 chunk（副本 failover）→ `Body::from_stream`；
  Range 仅拉取重叠 chunk 并裁剪。
- **ETag**：单 PUT `"{sha256hex}"`；multipart `"{sha256(part_etag_concat)}-{n}"`。
- raw_store gRPC 客户端/服务端 `max_frame_size = 16MB`（4MB chunk + 编码开销）。
- 节点数 < 副本数时降级；单副本读失败自动切换下一副本。

## 验证

> 按项目约定：代码完成后由用户手动编译，不主动执行 `cargo build`。

1. 用户编译：`cargo build --release`。
2. 本地多进程/容器联调：
   - Raft：现有 `raft-node-1..3`，在 `raft_service.groups` 中新增 `objmeta`、`raw_registry` 组；
   - raw_store：`raw-store-1..3`（不同 `root`、`node_id`，指向 Raft 注册）；
   - S3 网关：`s3-gw-1`（`raw_store_seeds` 指向 3 个 raw_store）。
3. 使用 aws cli / mc 验证：
   - `create-bucket`、`put-object`（10MB）、`get-object`、`head-object`；
   - `get-object --range bytes=100-1000`（206）；
   - `list-objects-v2`（prefix / delimiter / marker）；
   - `copy-object`、`delete-object`、`delete-objects`；
   - 大文件 `aws s3 cp`（触发 multipart）。
4. 故障演练：
   - 停 1 个 raw_store：读仍成功（剩余 2 副本）；
   - 停 2 个 raw_store：读失败，元数据仍可查；
   - 重启 raw_store：数据仍可读。
5. 回归检查：现有 19000 S3 端口、`/api/v1/...` REST、旧对象数据读写行为完全不变。
6. 落盘校验：raw_store `root` 下存在 chunk 文件，大小等于 4MB 或尾块大小。

## 不包含 / 后续

- 副本丢失后的自动 rebalance / 自愈；
- Raft 前缀扫描优化（本期用全量 Scan + 本地过滤）；
- S3 版本控制、ACL、生命周期、对象锁等高级特性；
- 现有单机对象存储的任何改动（零改动）。
