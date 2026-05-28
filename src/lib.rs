pub mod db;
pub mod rpc;
pub mod cli;

pub mod pb {
    tonic::include_proto!("laoflchdb.rpc");
    tonic::include_proto!("laoflchdb.metadata");
}

pub use db::OltpDB;
pub use cli::{Cli, Commands};
pub use rpc::LaoflchDbServiceImpl;

pub use db::{
    generate_table_uuid, generate_column_uuid,
    META_TABLE_PREFIX, META_COL_PREFIX, LAOFLCHDB_NAMESPACE,
};
