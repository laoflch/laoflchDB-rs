use crate::pb::{ColumnType, Field};
use prost::Message;

pub trait Column: Send + Sync {
    fn column_type(&self) -> ColumnType;
    fn encode(&self) -> Vec<u8>;
    fn decode(encoded: &[u8]) -> Result<Self, &'static str>
    where
        Self: Sized;
}

pub trait CString: Column + Send + Sync {
    fn get_value(&self) -> &str;
}

pub trait CInteger: Column + Send + Sync {
    fn get_value(&self) -> i64;
}

pub trait CBytes: Column + Send + Sync {
    fn get_value(&self) -> &[u8];
}

pub trait CFloat: Column + Send + Sync {
    fn get_value(&self) -> f64;
}

pub trait CList: Column + Send + Sync {
    fn get_items(&self) -> &[std::borrow::Cow<'static, [u8]>];
}

pub trait CImage: Column + Send + Sync {
    fn get_data(&self) -> &[u8];
    fn get_format(&self) -> &str;
}

pub struct StringColumn(pub String);

impl StringColumn {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}

impl Column for StringColumn {
    fn column_type(&self) -> ColumnType {
        ColumnType::String
    }

    fn encode(&self) -> Vec<u8> {
        let field = Field {
            value: Some(crate::pb::field::Value::StringValue(crate::pb::String {
                value: self.0.clone(),
            })),
        };
        field.encode_to_vec()
    }

    fn decode(encoded: &[u8]) -> Result<Self, &'static str> {
        let field = Field::decode(encoded).map_err(|_| "decode error")?;
        match field.value {
            Some(crate::pb::field::Value::StringValue(s)) => Ok(StringColumn(s.value)),
            _ => Err("not a string field"),
        }
    }
}

impl CString for StringColumn {
    fn get_value(&self) -> &str {
        &self.0
    }
}

pub struct IntegerColumn(pub i64);

impl IntegerColumn {
    pub fn new(value: i64) -> Self {
        Self(value)
    }
}

impl Column for IntegerColumn {
    fn column_type(&self) -> ColumnType {
        ColumnType::Int64
    }

    fn encode(&self) -> Vec<u8> {
        let field = Field {
            value: Some(crate::pb::field::Value::IntegerValue(crate::pb::Integer {
                value: self.0,
            })),
        };
        field.encode_to_vec()
    }

    fn decode(encoded: &[u8]) -> Result<Self, &'static str> {
        let field = Field::decode(encoded).map_err(|_| "decode error")?;
        match field.value {
            Some(crate::pb::field::Value::IntegerValue(i)) => Ok(IntegerColumn(i.value)),
            _ => Err("not an integer field"),
        }
    }
}

impl CInteger for IntegerColumn {
    fn get_value(&self) -> i64 {
        self.0
    }
}

pub struct BytesColumn(pub Vec<u8>);

impl BytesColumn {
    pub fn new(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl Column for BytesColumn {
    fn column_type(&self) -> ColumnType {
        ColumnType::Bytes
    }

    fn encode(&self) -> Vec<u8> {
        let field = Field {
            value: Some(crate::pb::field::Value::BytesValue(crate::pb::Bytes {
                value: self.0.clone(),
            })),
        };
        field.encode_to_vec()
    }

    fn decode(encoded: &[u8]) -> Result<Self, &'static str> {
        let field = Field::decode(encoded).map_err(|_| "decode error")?;
        match field.value {
            Some(crate::pb::field::Value::BytesValue(b)) => Ok(BytesColumn(b.value)),
            _ => Err("not a bytes field"),
        }
    }
}

impl CBytes for BytesColumn {
    fn get_value(&self) -> &[u8] {
        &self.0
    }
}

pub struct FloatColumn(pub f64);

impl FloatColumn {
    pub fn new(value: f64) -> Self {
        Self(value)
    }
}

impl Column for FloatColumn {
    fn column_type(&self) -> ColumnType {
        ColumnType::Float
    }

    fn encode(&self) -> Vec<u8> {
        let field = Field {
            value: Some(crate::pb::field::Value::FloatValue(crate::pb::Float {
                value: self.0,
            })),
        };
        field.encode_to_vec()
    }

    fn decode(encoded: &[u8]) -> Result<Self, &'static str> {
        let field = Field::decode(encoded).map_err(|_| "decode error")?;
        match field.value {
            Some(crate::pb::field::Value::FloatValue(f)) => Ok(FloatColumn(f.value)),
            _ => Err("not a float field"),
        }
    }
}

impl CFloat for FloatColumn {
    fn get_value(&self) -> f64 {
        self.0
    }
}

pub struct ListColumn {
    items: Vec<Vec<u8>>,
}

impl ListColumn {
    pub fn new(items: Vec<Vec<u8>>) -> Self {
        Self { items }
    }

    pub fn from_strings(items: Vec<String>) -> Self {
        Self { items: items.into_iter().map(|s| s.into_bytes()).collect() }
    }

    pub fn get_raw_items(&self) -> &[Vec<u8>] {
        &self.items
    }
}

impl Column for ListColumn {
    fn column_type(&self) -> ColumnType {
        ColumnType::List
    }

    fn encode(&self) -> Vec<u8> {
        let list = crate::pb::List {
            items: self.items.clone(),
        };
        let field = Field {
            value: Some(crate::pb::field::Value::ListValue(list)),
        };
        field.encode_to_vec()
    }

    fn decode(encoded: &[u8]) -> Result<Self, &'static str> {
        let field = Field::decode(encoded).map_err(|_| "decode error")?;
        match field.value {
            Some(crate::pb::field::Value::ListValue(l)) => Ok(ListColumn::new(l.items)),
            _ => Err("not a list field"),
        }
    }
}

impl CList for ListColumn {
    fn get_items(&self) -> &[std::borrow::Cow<'static, [u8]>] {
        &[]
    }
}

pub struct ImageColumn {
    pub data: Vec<u8>,
    pub format: String,
}

impl ImageColumn {
    pub fn new(data: Vec<u8>, format: String) -> Self {
        Self { data, format }
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            format: String::new(),
        }
    }
}

impl Column for ImageColumn {
    fn column_type(&self) -> ColumnType {
        ColumnType::Image
    }

    fn encode(&self) -> Vec<u8> {
        let image = crate::pb::Image {
            data: self.data.clone(),
            format: self.format.clone(),
        };
        let field = Field {
            value: Some(crate::pb::field::Value::ImageValue(image)),
        };
        field.encode_to_vec()
    }

    fn decode(encoded: &[u8]) -> Result<Self, &'static str> {
        let field = Field::decode(encoded).map_err(|_| "decode error")?;
        match field.value {
            Some(crate::pb::field::Value::ImageValue(img)) => Ok(ImageColumn {
                data: img.data,
                format: img.format,
            }),
            _ => Err("not an image field"),
        }
    }
}

impl CImage for ImageColumn {
    fn get_data(&self) -> &[u8] {
        &self.data
    }

    fn get_format(&self) -> &str {
        &self.format
    }
}
