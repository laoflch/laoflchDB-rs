use crate::service::DatabaseService;
use laoflchdb_engines::{ColumnType, Row, Field, SpecialFields, EnumOrUnknown, RowType};
use laoflchdb_engines::field::{String, Integer, Float};
use laoflchdb_engines::field::field::Value;
use laoflchdb_engines::Message;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

fn encode_field(f: &Field) -> Vec<u8> {
    let mut buf = Vec::new();
    f.write_to_vec(&mut buf).unwrap();
    buf
}

fn generate_id() -> u64 {
    let now = SystemTime::now();
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap();
    (since_epoch.as_secs() * 1000) + (since_epoch.subsec_nanos() / 1_000_000) as u64
}

pub async fn init_example_data(db_service: &crate::service::DatabaseServiceImpl) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("=== 初始化 laoflchDB 示例数据 ===");
    
    let schema = "example";
    
    let schemas = db_service.list_schemas().await?;
    let is_recreate = schemas.contains(&schema.to_string());
    if is_recreate {
        println!("⚠️ Schema '{}' 已存在，删除重建", schema);
        db_service.drop_schema(schema).await?;
        println!("✅ 删除 Schema '{}'", schema);
    }
    
    db_service.create_schema(schema).await?;
    println!("✅ 创建 Schema '{}'", schema);
    
    let existing_tables = if is_recreate {
        HashSet::new()
    } else {
        match db_service.list_tables(schema).await {
            Ok(tables) => tables.into_iter().collect::<HashSet<_>>(),
            Err(_) => HashSet::new(),
        }
    };
    
    println!("\n--- 创建客户表 customers ---");
    let customers_columns = [
        (1, "id", ColumnType::COLUMN_TYPE_INT64, Some("客户唯一标识，主键自增")),
        (2, "name", ColumnType::COLUMN_TYPE_STRING, Some("客户姓名")),
        (3, "email", ColumnType::COLUMN_TYPE_STRING, Some("客户邮箱地址")),
        (4, "phone", ColumnType::COLUMN_TYPE_STRING, Some("客户手机号码")),
        (5, "address", ColumnType::COLUMN_TYPE_STRING, Some("客户收货地址")),
        (6, "created_at", ColumnType::COLUMN_TYPE_STRING, Some("客户创建时间")),
    ];
    
    if !existing_tables.contains("customers") {
        let table_id = db_service.create_table(schema, "customers", Some("客户表：存储系统客户信息，包含客户基本资料"), &customers_columns).await?;
        println!("✅ 创建表 customers (ID: {})", table_id);
    } else {
        println!("⚠️ 表 'customers' 已存在");
    }
    
    println!("\n--- 创建产品表 products ---");
    let products_columns = [
        (1, "id", ColumnType::COLUMN_TYPE_INT64, Some("产品唯一标识，主键自增")),
        (2, "name", ColumnType::COLUMN_TYPE_STRING, Some("产品名称")),
        (3, "price", ColumnType::COLUMN_TYPE_FLOAT, Some("产品单价（元）")),
        (4, "stock", ColumnType::COLUMN_TYPE_INT64, Some("库存数量")),
        (5, "category", ColumnType::COLUMN_TYPE_STRING, Some("产品分类：电子产品、服装鞋帽、食品饮料、家居用品、图书文具")),
        (6, "description", ColumnType::COLUMN_TYPE_STRING, Some("产品描述信息")),
    ];
    
    if !existing_tables.contains("products") {
        let table_id = db_service.create_table(schema, "products", Some("产品表：存储商品信息，支持库存管理"), &products_columns).await?;
        println!("✅ 创建表 products (ID: {})", table_id);
    } else {
        println!("⚠️ 表 'products' 已存在");
    }
    
    println!("\n--- 创建订单表 orders ---");
    let orders_columns = [
        (1, "id", ColumnType::COLUMN_TYPE_INT64, Some("订单唯一标识，主键")),
        (2, "customer_id", ColumnType::COLUMN_TYPE_INT64, Some("客户ID，外键关联customers表")),
        (3, "order_date", ColumnType::COLUMN_TYPE_STRING, Some("下单时间")),
        (4, "total_amount", ColumnType::COLUMN_TYPE_FLOAT, Some("订单总金额（元）")),
        (5, "status", ColumnType::COLUMN_TYPE_STRING, Some("订单状态：PENDING-待支付、PAID-已支付、SHIPPED-已发货、COMPLETED-已完成、CANCELLED-已取消")),
        (6, "shipping_address", ColumnType::COLUMN_TYPE_STRING, Some("配送地址")),
    ];
    
    if !existing_tables.contains("orders") {
        let table_id = db_service.create_table(schema, "orders", Some("订单表：存储客户订单信息，记录订单的完整生命周期"), &orders_columns).await?;
        println!("✅ 创建表 orders (ID: {})", table_id);
    } else {
        println!("⚠️ 表 'orders' 已存在");
    }
    
    println!("\n--- 创建订单明细表 order_items ---");
    let order_items_columns = [
        (1, "id", ColumnType::COLUMN_TYPE_INT64, Some("订单明细唯一标识，主键")),
        (2, "order_id", ColumnType::COLUMN_TYPE_INT64, Some("订单ID，外键关联orders表")),
        (3, "product_id", ColumnType::COLUMN_TYPE_INT64, Some("产品ID，外键关联products表")),
        (4, "quantity", ColumnType::COLUMN_TYPE_INT64, Some("购买数量")),
        (5, "unit_price", ColumnType::COLUMN_TYPE_FLOAT, Some("商品单价（元）")),
        (6, "discount", ColumnType::COLUMN_TYPE_FLOAT, Some("折扣率（0.0-1.0）")),
    ];
    
    if !existing_tables.contains("order_items") {
        let table_id = db_service.create_table(schema, "order_items", Some("订单明细表：存储订单中的商品明细，支持一个订单包含多个商品"), &order_items_columns).await?;
        println!("✅ 创建表 order_items (ID: {})", table_id);
    } else {
        println!("⚠️ 表 'order_items' 已存在");
    }
    
    println!("\n--- 插入客户数据 (1000条) ---");
    let customer_names = [
        "张三", "李四", "王五", "赵六", "钱七", "孙八", "周九", "吴十",
        "郑一", "陈二", "黄三", "刘四", "杨五", "赵六", "徐七", "马八",
        "朱九", "胡十", "林一", "何二", "梁三", "郭四", "罗五", "高六",
        "林七", "谢八", "宋九", "唐十", "许一", "邓二", "曹三", "彭四",
    ];
    
    let emails = [
        "gmail.com", "qq.com", "163.com", "126.com", "sina.com", 
        "hotmail.com", "outlook.com", "icloud.com", "me.com", "aliyun.com"
    ];
    
    let phones = ["138", "139", "158", "159", "186", "188", "136", "137"];
    let addresses = [
        "北京市朝阳区", "上海市浦东新区", "广州市天河区", "深圳市南山区", 
        "杭州市西湖区", "南京市鼓楼区", "成都市锦江区", "武汉市洪山区",
        "西安市雁塔区", "重庆市渝北区"
    ];
    
    for i in 0..1000 {
        let id = i as i64 + 1;
        let name = format!("{}", customer_names[i % customer_names.len()]);
        let email = format!("user{}@{}", i, emails[i % emails.len()]);
        let phone = format!("{}{:08}", phones[i % phones.len()], i);
        let address = format!("{}第{}号", addresses[i % addresses.len()], i % 100 + 1);
        let created_at = format!("2026-0{}-{:02} {:02}:{:02}:{:02}", 
            (i % 12) + 1, (i % 28) + 1, (i % 24), (i % 60), (i % 60));
        
        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                encode_field(&Field {
                    value: Some(Value::IntegerValue(Integer { value: id, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: name, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: email, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: phone, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: address, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: created_at, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
            ],
            special_fields: SpecialFields::default(),
        };
        
        db_service.add_row(schema, "customers", &row).await?;
        
        if (i + 1) % 200 == 0 {
            println!("  已插入 {} 条客户数据", i + 1);
        }
    }
    println!("✅ 完成插入 1000 条客户数据");
    
    println!("\n--- 插入产品数据 (100条) ---");
    let _categories = ["电子产品", "服装鞋帽", "食品饮料", "家居用品", "图书文具"];
    let product_names = [
        ("iPhone 15 Pro", 8999.00, "电子产品", "苹果手机"),
        ("MacBook Pro 14", 14999.00, "电子产品", "苹果笔记本"),
        ("AirPods Pro 2", 1899.00, "电子产品", "苹果耳机"),
        ("iPad Air", 5999.00, "电子产品", "苹果平板"),
        ("Apple Watch", 3999.00, "电子产品", "苹果手表"),
        ("华为 Mate 60 Pro", 6999.00, "电子产品", "华为手机"),
        ("小米 14 Ultra", 5999.00, "电子产品", "小米手机"),
        ("索尼 WH-1000XM5", 2999.00, "电子产品", "索尼耳机"),
        ("任天堂 Switch", 2099.00, "电子产品", "游戏主机"),
        ("PS5 Slim", 4299.00, "电子产品", "游戏主机"),
        ("Nike Air Force 1", 799.00, "服装鞋帽", "运动鞋"),
        ("Adidas Superstar", 699.00, "服装鞋帽", "运动鞋"),
        ("优衣库羽绒服", 599.00, "服装鞋帽", "羽绒服"),
        ("Zara 牛仔裤", 299.00, "服装鞋帽", "牛仔裤"),
        ("Levis 牛仔裤", 799.00, "服装鞋帽", "牛仔裤"),
        ("三只松鼠坚果礼盒", 199.00, "食品饮料", "坚果"),
        ("农夫山泉矿泉水", 2.00, "食品饮料", "矿泉水"),
        ("蒙牛纯牛奶", 68.00, "食品饮料", "牛奶"),
        ("可口可乐", 3.50, "食品饮料", "饮料"),
        ("旺旺雪饼", 18.00, "食品饮料", "零食"),
        ("宜家书架", 499.00, "家居用品", "书架"),
        ("小米台灯", 199.00, "家居用品", "台灯"),
        ("美的空调", 2999.00, "家居用品", "空调"),
        ("海尔冰箱", 3999.00, "家居用品", "冰箱"),
        ("九阳豆浆机", 399.00, "家居用品", "豆浆机"),
        ("Python编程从入门到精通", 89.00, "图书文具", "编程书籍"),
        ("三体全集", 99.00, "图书文具", "科幻小说"),
        ("现代汉语词典", 129.00, "图书文具", "词典"),
        ("晨光中性笔", 2.00, "图书文具", "文具"),
        ("得力文件夹", 15.00, "图书文具", "文具"),
    ];
    
    for i in 0..100 {
        let base_idx = i % product_names.len();
        let (name, base_price, category, desc) = product_names[base_idx];
        let id = i as i64 + 1;
        let price = base_price + (i % 10) as f64 * 10.0;
        let stock = ((i % 50) + 50) as i64;
        let description = format!("{}-{}", desc, i + 1);
        
        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                encode_field(&Field {
                    value: Some(Value::IntegerValue(Integer { value: id, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: name.to_string(), special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::FloatValue(Float { value: price, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::IntegerValue(Integer { value: stock, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: category.to_string(), special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: description, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
            ],
            special_fields: SpecialFields::default(),
        };
        
        db_service.add_row(schema, "products", &row).await?;
        
        if (i + 1) % 20 == 0 {
            println!("  已插入 {} 条产品数据", i + 1);
        }
    }
    println!("✅ 完成插入 100 条产品数据");
    
    println!("\n--- 插入订单数据 (10000条) ---");
    let statuses = ["PENDING", "PAID", "SHIPPED", "COMPLETED", "CANCELLED"];
    
    for i in 0..10000 {
        let order_id = generate_id() + i as u64;
        let customer_id = ((i % 1000) + 1) as i64;
        let month = (i % 12) + 1;
        let day = (i % 28) + 1;
        let order_date = format!("2026-0{}-{:02} {:02}:{:02}:{:02}", month, day, (i % 24), (i % 60), (i % 60));
        let status = statuses[i % statuses.len()];
        let address = format!("{}第{}号", addresses[i % addresses.len()], (i % 100) + 1);
        
        let num_items = (i % 4) + 1;
        let mut total_amount = 0.0;
        
        for j in 0..num_items {
            let product_id = ((i * 7 + j * 13) % 100) + 1;
            let quantity = ((i % 5) + 1) as i64;
            let unit_price = 100.0 + (i % 100) as f64 * 50.0;
            let discount = if i % 10 == 0 { 0.1 } else { 0.0 };
            
            total_amount += unit_price * quantity as f64 * (1.0 - discount);
            
            let item_row = Row {
                row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
                version: 1,
                data: vec![
                    encode_field(&Field {
                        value: Some(Value::IntegerValue(Integer { value: (order_id + j as u64) as i64, special_fields: SpecialFields::default() })),
                        special_fields: SpecialFields::default(),
                    }),
                    encode_field(&Field {
                        value: Some(Value::IntegerValue(Integer { value: order_id as i64, special_fields: SpecialFields::default() })),
                        special_fields: SpecialFields::default(),
                    }),
                    encode_field(&Field {
                        value: Some(Value::IntegerValue(Integer { value: product_id as i64, special_fields: SpecialFields::default() })),
                        special_fields: SpecialFields::default(),
                    }),
                    encode_field(&Field {
                        value: Some(Value::IntegerValue(Integer { value: quantity, special_fields: SpecialFields::default() })),
                        special_fields: SpecialFields::default(),
                    }),
                    encode_field(&Field {
                        value: Some(Value::FloatValue(Float { value: unit_price, special_fields: SpecialFields::default() })),
                        special_fields: SpecialFields::default(),
                    }),
                    encode_field(&Field {
                        value: Some(Value::FloatValue(Float { value: discount, special_fields: SpecialFields::default() })),
                        special_fields: SpecialFields::default(),
                    }),
                ],
                special_fields: SpecialFields::default(),
            };
            
            db_service.add_row(schema, "order_items", &item_row).await?;
        }
        
        let order_row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                encode_field(&Field {
                    value: Some(Value::IntegerValue(Integer { value: order_id as i64, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::IntegerValue(Integer { value: customer_id, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: order_date, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::FloatValue(Float { value: total_amount, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: status.to_string(), special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
                encode_field(&Field {
                    value: Some(Value::StringValue(String { value: address, special_fields: SpecialFields::default() })),
                    special_fields: SpecialFields::default(),
                }),
            ],
            special_fields: SpecialFields::default(),
        };
        
        db_service.add_row(schema, "orders", &order_row).await?;
        
        if (i + 1) % 1000 == 0 {
            println!("  已插入 {} 条订单数据", i + 1);
        }
    }
    println!("✅ 完成插入 10000 条订单数据");
    
    println!("\n🎉 Example 数据库初始化完成！");
    println!("   ├── 客户表 (customers): 1000 条");
    println!("   ├── 产品表 (products): 100 条");
    println!("   ├── 订单表 (orders): 10000 条");
    println!("   └── 订单明细表 (order_items): ~20000 条");
    Ok(())
}