//! 跨 Schema JOIN 集成测试
//! 
//! 测试不同 schema 之间的表 JOIN 操作

use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_engines::{Row, RowType, SpecialFields, Message, field::field::Value, field::{String, Integer, Float}};
use protobuf::CodedOutputStream;
use std::sync::Arc;

/// 创建测试服务
async fn create_test_service() -> Arc<dyn DatabaseService> {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_cross_schema_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = DatabaseServiceImpl::new(db_path_str).await;
    service.init_database().await.unwrap();
    Arc::new(service)
}

/// 辅助函数：创建表并插入数据
async fn create_and_populate_table(
    service: &Arc<dyn DatabaseService>,
    schema: &str,
    table_name: &str,
    columns: Vec<(u32, &str, laoflchdb_engines::ColumnType)>,
    data: Vec<Vec<(laoflchdb_engines::ColumnType, &str)>>,
) {
    let column_tuples: Vec<(u32, &str, laoflchdb_engines::ColumnType, Option<&str>)> = 
        columns.into_iter().map(|(i, name, col_type)| (i, name, col_type, None)).collect();
    
    service.create_table(schema, table_name, None, &column_tuples).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    for row_data in data {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        for (col_type, value) in row_data {
            let mut f = laoflchdb_engines::Field::new();
            match col_type {
                laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64 => {
                    f.value = Some(Value::IntegerValue(Integer { 
                        value: value.parse::<i64>().unwrap(), 
                        special_fields: SpecialFields::default() 
                    }));
                }
                laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING => {
                    f.value = Some(Value::StringValue(String { 
                        value: value.to_string(), 
                        special_fields: SpecialFields::default() 
                    }));
                }
                laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT => {
                    f.value = Some(Value::FloatValue(Float { 
                        value: value.parse::<f64>().unwrap(), 
                        special_fields: SpecialFields::default() 
                    }));
                }
                _ => {}
            }
            let mut buf = Vec::new();
            { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
            row.data.push(buf);
        }
        
        service.add_row(schema, table_name, &row).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}

/// 测试跨 Schema JOIN 基本功能
#[tokio::test]
async fn test_cross_schema_join_basic() {
    let service = create_test_service().await;
    
    // 在 sales schema 中创建 orders 表
    let sales_columns = vec![
        (0, "order_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "customer_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (2, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (3, "amount", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT),
    ];
    
    let sales_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "101"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "10"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "99.99"),
        ],
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "102"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "2"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "20"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "199.99"),
        ],
    ];
    
    create_and_populate_table(&service, "sales", "orders", sales_columns, sales_data).await;
    
    // 在 inventory schema 中创建 products 表
    let inventory_columns = vec![
        (0, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "product_name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
        (2, "price", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT),
    ];
    
    let inventory_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "10"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Laptop"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "999.99"),
        ],
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "20"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Monitor"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "199.99"),
        ],
    ];
    
    create_and_populate_table(&service, "inventory", "products", inventory_columns, inventory_data).await;
    
    // 测试跨 schema INNER JOIN
    let result = service.sql_query(
        "sales", 
        "SELECT sales.orders.order_id, inventory.products.product_name 
         FROM sales.orders 
         JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id"
    ).await;
    
    assert!(result.is_ok(), "跨 schema JOIN 查询失败");
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 2, "跨 schema JOIN 应返回 2 行");
    
    println!("跨 Schema JOIN 基本测试通过!");
}

/// 测试跨 Schema LEFT JOIN
#[tokio::test]
async fn test_cross_schema_left_join() {
    let service = create_test_service().await;
    
    // 创建 sales.orders 表
    let sales_columns = vec![
        (0, "order_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (2, "amount", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT),
    ];
    
    let sales_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "100.0"),
        ],
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "2"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "99"),  // 不存在的 product_id
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "50.0"),
        ],
    ];
    
    create_and_populate_table(&service, "sales", "orders", sales_columns, sales_data).await;
    
    // 创建 inventory.products 表
    let inventory_columns = vec![
        (0, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "product_name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
    ];
    
    let inventory_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Product A"),
        ],
    ];
    
    create_and_populate_table(&service, "inventory", "products", inventory_columns, inventory_data).await;
    
    // 测试跨 schema LEFT JOIN
    let result = service.sql_query(
        "sales", 
        "SELECT sales.orders.order_id, inventory.products.product_name 
         FROM sales.orders 
         LEFT JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id"
    ).await;
    
    assert!(result.is_ok(), "跨 schema LEFT JOIN 查询失败");
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 2, "跨 schema LEFT JOIN 应返回 2 行（包括不匹配的行）");
    
    println!("跨 Schema LEFT JOIN 测试通过!");
}

/// 测试三表跨 Schema JOIN
#[tokio::test]
async fn test_cross_schema_triple_join() {
    let service = create_test_service().await;
    
    // 创建 sales.customers 表
    let customer_columns = vec![
        (0, "customer_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "customer_name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
    ];
    
    let customer_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Alice"),
        ],
    ];
    
    create_and_populate_table(&service, "sales", "customers", customer_columns, customer_data).await;
    
    // 创建 sales.orders 表
    let order_columns = vec![
        (0, "order_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "customer_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (2, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
    ];
    
    let order_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "101"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "10"),
        ],
    ];
    
    create_and_populate_table(&service, "sales", "orders", order_columns, order_data).await;
    
    // 创建 inventory.products 表
    let product_columns = vec![
        (0, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "product_name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
        (2, "price", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT),
    ];
    
    let product_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "10"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Laptop"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "999.99"),
        ],
    ];
    
    create_and_populate_table(&service, "inventory", "products", product_columns, product_data).await;
    
    // 测试三表跨 schema JOIN
    let result = service.sql_query(
        "sales", 
        "SELECT sales.customers.customer_name, inventory.products.product_name, sales.orders.order_id 
         FROM sales.customers 
         JOIN sales.orders ON sales.customers.customer_id = sales.orders.customer_id 
         JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id"
    ).await;
    
    assert!(result.is_ok(), "三表跨 schema JOIN 查询失败");
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1, "三表跨 schema JOIN 应返回 1 行");
    
    println!("三表跨 Schema JOIN 测试通过!");
}

/// 测试跨 Schema JOIN 带 WHERE 条件
#[tokio::test]
async fn test_cross_schema_join_with_where() {
    let service = create_test_service().await;
    
    // 创建 sales.orders 表
    let order_columns = vec![
        (0, "order_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (2, "amount", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT),
    ];
    
    let order_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "100.0"),
        ],
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "2"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "2"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "200.0"),
        ],
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "3"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, "150.0"),
        ],
    ];
    
    create_and_populate_table(&service, "sales", "orders", order_columns, order_data).await;
    
    // 创建 inventory.products 表
    let product_columns = vec![
        (0, "product_id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1, "product_name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
        (2, "category", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
    ];
    
    let product_data = vec![
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "1"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Electronics"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Electronics"),
        ],
        vec![
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, "2"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Books"),
            (laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, "Books"),
        ],
    ];
    
    create_and_populate_table(&service, "inventory", "products", product_columns, product_data).await;
    
    // 测试跨 schema JOIN 带 WHERE 条件
    let result = service.sql_query(
        "sales", 
        "SELECT sales.orders.order_id, inventory.products.product_name 
         FROM sales.orders 
         JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id 
         WHERE inventory.products.category = 'Electronics'"
    ).await;
    
    assert!(result.is_ok(), "跨 schema JOIN 带 WHERE 查询失败");
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 2, "跨 schema JOIN 带 WHERE 应返回 2 行");
    
    println!("跨 Schema JOIN 带 WHERE 条件测试通过!");
}
