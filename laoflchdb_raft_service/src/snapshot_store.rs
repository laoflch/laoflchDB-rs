//! 持久化快照存储
//!
//! 修复历史缺陷：原实现 `build_snapshot` 只生成内存快照、从不落盘，
//! 而 `RaftLogStore::purge` 又把日志（含成员配置条目）删掉，
//! 导致节点重启后 openraft 无法从日志/快照恢复成员关系，
//! 组退化为 `Learner` + 空 membership，永远选不出 leader
//! （现象即写入报 `has to forward request to: None, None`）。
//!
//! 本模块把快照元信息（`SnapshotMeta`，含 last_log_id 与 last_membership）与
//! 快照数据一起落盘到组目录（`data_path/group-{id}`），启动时可恢复。

use crate::raft_types::NodeId;
use openraft::{BasicNode, SnapshotMeta};
use std::fs;
use std::path::{Path, PathBuf};

const META_FILE: &str = "snapshot.meta.json";
const DATA_FILE: &str = "snapshot.data";
const TMP_SUFFIX: &str = ".tmp";

/// 组级快照存储
#[derive(Clone, Debug)]
pub struct SnapshotStore {
    dir: PathBuf,
}

impl SnapshotStore {
    pub fn new<P: AsRef<Path>>(dir: P) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 读取快照元信息（不存在或损坏返回 None）
    pub fn load_meta(&self) -> Option<SnapshotMeta<NodeId, BasicNode>> {
        let blob = fs::read(self.dir.join(META_FILE)).ok()?;
        serde_json::from_slice(&blob).ok()
    }

    /// 已持久化快照覆盖到的最后一个日志索引
    pub fn last_index(&self) -> Option<u64> {
        self.load_meta().and_then(|m| m.last_log_id.map(|l| l.index))
    }

    /// 读取完整快照（元信息 + 数据）
    pub fn load(&self) -> Option<(SnapshotMeta<NodeId, BasicNode>, Vec<u8>)> {
        let meta = self.load_meta()?;
        let data = fs::read(self.dir.join(DATA_FILE)).ok()?;
        Some((meta, data))
    }

    /// 落盘快照：先写数据再写元信息，元信息最后原子重命名，作为“快照有效”的提交点
    pub fn save(
        &self,
        meta: &SnapshotMeta<NodeId, BasicNode>,
        data: &[u8],
    ) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("创建快照目录失败: {e}"))?;

        let data_path = self.dir.join(DATA_FILE);
        let data_tmp = self.dir.join(format!("{DATA_FILE}{TMP_SUFFIX}"));
        fs::write(&data_tmp, data).map_err(|e| format!("写入快照数据失败: {e}"))?;
        fs::rename(&data_tmp, &data_path).map_err(|e| format!("提交快照数据失败: {e}"))?;

        let meta_blob = serde_json::to_vec(meta)
            .map_err(|e| format!("序列化快照元信息失败: {e}"))?;
        let meta_path = self.dir.join(META_FILE);
        let meta_tmp = self.dir.join(format!("{META_FILE}{TMP_SUFFIX}"));
        fs::write(&meta_tmp, &meta_blob).map_err(|e| format!("写入快照元信息失败: {e}"))?;
        fs::rename(&meta_tmp, &meta_path).map_err(|e| format!("提交快照元信息失败: {e}"))?;

        Ok(())
    }
}
