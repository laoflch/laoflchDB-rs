
use crate::service::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_db_engine::{ColumnType, Row, Field, SpecialFields, EnumOrUnknown, RowType};
use laoflchdb_db_engine::field::{String, Integer, Float};
use laoflchdb_db_engine::field::field::Value;
use laoflchdb_db_engine::Message;
use std::collections::HashSet;

fn encode_field(f: &Field) -> Vec<u8> {
    let mut buf = Vec::new();
    f.write_to_vec(&mut buf).unwrap();
    buf
}

pub async fn init_example_data(db_path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let db_service = DatabaseServiceImpl::new(db_path).await;
    
    println!("=== 初始化 laoflchDB 示例数据 ===");
    
    let schema = "example";
    
    match db_service.create_schema(schema).await {
        Ok(_) => println!("✅ 创建 Schema 'example'"),
        Err(e) => println!("⚠️ Schema 'example' 已存在: {}", e),
    }
    
    println!("\n--- 创建用户表 users ---");
    let users_columns = [
        (1, "id", ColumnType::COLUMN_TYPE_INT64),
        (2, "name", ColumnType::COLUMN_TYPE_STRING),
        (3, "age", ColumnType::COLUMN_TYPE_INT64),
        (4, "email", ColumnType::COLUMN_TYPE_STRING),
    ];
    
    let existing_tables = match db_service.list_tables(schema).await {
        Ok(tables) => tables.into_iter().collect::<HashSet<_>>(),
        Err(_) => HashSet::new(),
    };
    
    if !existing_tables.contains("users") {
        let table_id = db_service.create_table(schema, "users", &users_columns).await?;
        println!("✅ 创建表 users (ID: {})", table_id);
    } else {
        println!("⚠️ 表 'users' 已存在");
    }
    
    println!("\n--- 创建产品表 products ---");
    let products_columns = [
        (1, "id", ColumnType::COLUMN_TYPE_INT64),
        (2, "name", ColumnType::COLUMN_TYPE_STRING),
        (3, "price", ColumnType::COLUMN_TYPE_FLOAT),
        (4, "stock", ColumnType::COLUMN_TYPE_INT64),
    ];
    
    if !existing_tables.contains("products") {
        let table_id = db_service.create_table(schema, "products", &products_columns).await?;
        println!("✅ 创建表 products (ID: {})", table_id);
    } else {
        println!("⚠️ 表 'products' 已存在");
    }
    
    println!("\n--- 插入用户数据 ---");
    let users = [
        (1, "Alice", 30, "alice@example.com"),
        (2, "Bob", 25, "bob@example.com"),
        (3, "Charlie", 35, "charlie@example.com"),
        (4, "David", 28, "david@example.com"),
        (5, "Eve", 32, "eve@example.com"),
    ];
    
    for (id, name, age, email) in users {
        let id_field = Field {
            value: Some(Value::IntegerValue(Integer { 
                value: id as i64,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let name_field = Field {
            value: Some(Value::StringValue(String { 
                value: name.to_string(),
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let age_field = Field {
            value: Some(Value::IntegerValue(Integer { 
                value: age as i64,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let email_field = Field {
            value: Some(Value::StringValue(String { 
                value: email.to_string(),
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        
        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                encode_field(&id_field),
                encode_field(&name_field),
                encode_field(&age_field),
                encode_field(&email_field),
            ],
            special_fields: SpecialFields::default(),
        };
        
        let row_id = db_service.add_row(schema, "users", &row).await?;
        println!("✅ 插入用户 {} (Row ID: {})", name, row_id);
    }
    
    println!("\n--- 插入产品数据 ---");
    let products = [
        (1, "iPhone 15", 799.99, 100),
        (2, "MacBook Pro", 1999.99, 50),
        (3, "AirPods Pro", 249.00, 200),
        (4, "iPad Air", 599.00, 75),
        (5, "Apple Watch", 399.00, 120),
    ];
    
    for (id, name, price, stock) in products {
        let id_field = Field {
            value: Some(Value::IntegerValue(Integer { 
                value: id as i64,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let name_field = Field {
            value: Some(Value::StringValue(String { 
                value: name.to_string(),
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let price_field = Field {
            value: Some(Value::FloatValue(Float { 
                value: price,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let stock_field = Field {
            value: Some(Value::IntegerValue(Integer { 
                value: stock as i64,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        
        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                encode_field(&id_field),
                encode_field(&name_field),
                encode_field(&price_field),
                encode_field(&stock_field),
            ],
            special_fields: SpecialFields::default(),
        };
        
        let row_id = db_service.add_row(schema, "products", &row).await?;
        println!("✅ 插入产品 {} (Row ID: {})", name, row_id);
    }
    
    println!("\n🎉 Example 数据库初始化完成！");
    Ok(())
}
