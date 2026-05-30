use crate::pb::{Row, RowType, Field};
use prost::Message;

pub const DEFAULT_ROW_VERSION: u32 = 1;

pub type ColumnId = u64;
pub type ColumnData = Vec<u8>;

#[derive(Debug, Clone)]
pub enum FetchedRow {
    Normal(Row),
    Raw(Vec<u8>),
}

pub fn create_normal_row(data: Vec<Vec<u8>>) -> Row {
    Row {
        row_type: RowType::Normal.into(),
        version: DEFAULT_ROW_VERSION,
        data,
    }
}

pub fn create_normal_row_from_column_pairs(pairs: &[(ColumnId, ColumnData)]) -> Row {
    let max_col_id = pairs.iter().map(|(cid, _)| *cid).max().unwrap_or(0);
    let mut data = Vec::with_capacity((max_col_id + 1) as usize);
    data.resize((max_col_id + 1) as usize, Vec::new());
    
    for (cid, val) in pairs.iter() {
        data[*cid as usize] = val.clone();
    }
    
    Row {
        row_type: RowType::Normal.into(),
        version: DEFAULT_ROW_VERSION,
        data,
    }
}

pub fn normal_row_get_data_for_column_id<'a>(row: &'a Row, column_id: u64) -> Option<&'a [u8]> {
    let idx = column_id as usize;
    row.data.get(idx).filter(|d| !d.is_empty()).map(|v| &v[..])
}

pub fn normal_row_column_data_at(row: &Row, index: usize) -> Option<&[u8]> {
    row.data.get(index).filter(|d| !d.is_empty()).map(|v| &v[..])
}

pub fn normal_row_iter_columns(row: &Row) -> impl Iterator<Item = (u64, &[u8])> {
    row.data.iter().enumerate().filter(|(_, d)| !d.is_empty()).map(|(idx, d)| (idx as u64, &d[..]))
}

pub fn encode_row_to_bytes(row: &Row) -> Vec<u8> {
    row.encode_to_vec()
}

pub fn decode_row_from_bytes(bytes: &[u8]) -> Result<Row, prost::DecodeError> {
    Row::decode(bytes)
}

pub fn is_row_type_normal(row: &Row) -> bool {
    matches!(RowType::try_from(row.row_type), Ok(RowType::Normal))
}

pub fn create_raw_row_from_str(raw_content: &str) -> Row {
    create_raw_row(raw_content.as_bytes())
}

pub fn create_raw_row(raw_binary: &[u8]) -> Row {
    Row {
        row_type: RowType::Raw.into(),
        version: DEFAULT_ROW_VERSION,
        data: vec![raw_binary.to_vec()],
    }
}

pub fn is_row_type_raw(row: &Row) -> bool {
    matches!(RowType::try_from(row.row_type), Ok(RowType::Raw))
}

pub fn get_raw_row_content(row: &Row) -> Option<&[u8]> {
    if is_row_type_raw(row) && !row.data.is_empty() {
        Some(&row.data[0])
    } else {
        None
    }
}

pub fn get_raw_row_content_as_utf8(row: &Row) -> Result<Option<&str>, std::str::Utf8Error> {
    if let Some(bytes) = get_raw_row_content(row) {
        Ok(Some(std::str::from_utf8(bytes)?))
    } else {
        Ok(None)
    }
}

pub fn encode_field_to_bytes(field: &Field) -> Vec<u8> {
    field.encode_to_vec()
}

pub fn decode_field_from_bytes(bytes: &[u8]) -> Result<Field, prost::DecodeError> {
    Field::decode(bytes)
}
