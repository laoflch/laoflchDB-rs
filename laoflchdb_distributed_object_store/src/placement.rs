//! 一致性哈希环

use sha2::{Digest, Sha256};

pub fn hash64(data: &[u8]) -> u64 {
    let digest = Sha256::digest(data);
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&digest[0..8]);
    u64::from_le_bytes(buf)
}

/// 每个 raw_store 地址映射为多个虚拟节点，chunk 顺时针取副本。
pub struct ConsistentHashRing {
    points: Vec<(u64, String)>,
}

impl ConsistentHashRing {
    pub fn new(addrs: &[String], vnodes: usize) -> Self {
        let vnodes = vnodes.max(1);
        let mut points = Vec::with_capacity(addrs.len() * vnodes);
        for addr in addrs {
            for i in 0..vnodes {
                let h = hash64(format!("{addr}#{i}").as_bytes());
                points.push((h, addr.clone()));
            }
        }
        points.sort_by_key(|(h, _)| *h);
        Self { points }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// 返回顺时针方向的 count 个不同节点
    pub fn replicas(&self, key: &str, count: usize) -> Vec<String> {
        if self.points.is_empty() || count == 0 {
            return Vec::new();
        }

        let h = hash64(key.as_bytes());
        let n = self.points.len();
        let mut idx = self.points.partition_point(|(p, _)| *p < h) % n;
        let mut visited = 0usize;
        let mut result: Vec<String> = Vec::new();

        while visited < n && result.len() < count {
            let addr = self.points[idx].1.clone();
            if !result.iter().any(|a| a == &addr) {
                result.push(addr);
            }
            idx = (idx + 1) % n;
            visited += 1;
        }
        result
    }
}
