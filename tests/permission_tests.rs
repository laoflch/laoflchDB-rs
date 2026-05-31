use laoflchDB_rust::access::permission::{PermissionChecker, PermissionContext};
use laoflchDB_rust::config::{PermissionAction, ServicePermission, TablePermissions, DatabaseConfig};

#[test]
fn test_permission_checker_default_allow() {
    let checker = PermissionChecker::new(true);
    
    let context = PermissionContext {
        schema: "sys".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result = checker.check_permission("unknown_service", &context);
    assert!(result.allowed, "Unknown service should use global default (allow)");
}

#[test]
fn test_permission_checker_default_deny() {
    let checker = PermissionChecker::new(false);
    
    let context = PermissionContext {
        schema: "sys".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result = checker.check_permission("unknown_service", &context);
    assert!(!result.allowed, "Unknown service should use global default (deny)");
}

#[test]
fn test_service_explicit_allow() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "test_service".to_string(),
        default_policy: "deny".to_string(),
        allowed_actions: vec![PermissionAction::Get, PermissionAction::Put],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let context = PermissionContext {
        schema: "sys".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result = checker.check_permission("test_service", &context);
    assert!(result.allowed, "Service should have explicit allow for Get action");
}

#[test]
fn test_service_explicit_deny() {
    let mut checker = PermissionChecker::new(true);
    
    let permission = ServicePermission {
        service_id: "test_service".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![],
        denied_actions: vec![PermissionAction::Delete],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let context = PermissionContext {
        schema: "sys".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Delete,
    };
    
    let result = checker.check_permission("test_service", &context);
    assert!(!result.allowed, "Service should have explicit deny for Delete action");
}

#[test]
fn test_service_default_policy_when_not_explicit() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "test_service".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![PermissionAction::Get],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let context = PermissionContext {
        schema: "sys".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Delete,
    };
    
    let result = checker.check_permission("test_service", &context);
    assert!(result.allowed, "Should use service default policy (allow) when action not explicitly configured");
}

#[test]
fn test_wildcard_action_allows_all() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "admin".to_string(),
        default_policy: "deny".to_string(),
        allowed_actions: vec![PermissionAction::All],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let actions = vec![
        PermissionAction::Get,
        PermissionAction::Put,
        PermissionAction::Delete,
        PermissionAction::CreateTable,
    ];
    
    for action in actions {
        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("users".to_string()),
            action: action.clone(),
        };
        
        let result = checker.check_permission("admin", &context);
        assert!(result.allowed, "Wildcard action should allow all operations");
    }
}

#[test]
fn test_table_permissions_allowed_schemas() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "limited_service".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![PermissionAction::Get],
        denied_actions: vec![],
        table_permissions: Some(TablePermissions {
            allowed_schemas: vec!["public".to_string()],
            denied_schemas: vec![],
            allowed_tables: vec![],
            denied_tables: vec![],
        }),
    };
    
    checker.add_service_permission(permission);
    
    let context = PermissionContext {
        schema: "public".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result = checker.check_permission("limited_service", &context);
    assert!(result.allowed, "Should be allowed for public schema");
    
    let context_denied = PermissionContext {
        schema: "internal".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result_denied = checker.check_permission("limited_service", &context_denied);
    assert!(!result_denied.allowed, "Should be denied for non-allowed schema");
}

#[test]
fn test_table_permissions_denied_schemas() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "safe_service".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![PermissionAction::Get],
        denied_actions: vec![],
        table_permissions: Some(TablePermissions {
            allowed_schemas: vec![],
            denied_schemas: vec!["internal".to_string(), "admin".to_string()],
            allowed_tables: vec![],
            denied_tables: vec![],
        }),
    };
    
    checker.add_service_permission(permission);
    
    let context = PermissionContext {
        schema: "internal".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result = checker.check_permission("safe_service", &context);
    assert!(!result.allowed, "Should be denied for internal schema");
}

#[test]
fn test_table_permissions_allowed_tables() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "user_service".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![PermissionAction::Get],
        denied_actions: vec![],
        table_permissions: Some(TablePermissions {
            allowed_schemas: vec![],
            denied_schemas: vec![],
            allowed_tables: vec!["users".to_string(), "products".to_string()],
            denied_tables: vec![],
        }),
    };
    
    checker.add_service_permission(permission);
    
    let context_allowed = PermissionContext {
        schema: "sys".to_string(),
        table: Some("users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result_allowed = checker.check_permission("user_service", &context_allowed);
    assert!(result_allowed.allowed, "Should be allowed for users table");
    
    let context_denied = PermissionContext {
        schema: "sys".to_string(),
        table: Some("admin_users".to_string()),
        action: PermissionAction::Get,
    };
    
    let result_denied = checker.check_permission("user_service", &context_denied);
    assert!(!result_denied.allowed, "Should be denied for admin_users table");
}

#[test]
fn test_table_permissions_denied_tables() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "general_service".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![PermissionAction::Get],
        denied_actions: vec![],
        table_permissions: Some(TablePermissions {
            allowed_schemas: vec![],
            denied_schemas: vec![],
            allowed_tables: vec![],
            denied_tables: vec!["secrets".to_string(), "passwords".to_string()],
        }),
    };
    
    checker.add_service_permission(permission);
    
    let context_denied = PermissionContext {
        schema: "sys".to_string(),
        table: Some("secrets".to_string()),
        action: PermissionAction::Get,
    };
    
    let result_denied = checker.check_permission("general_service", &context_denied);
    assert!(!result_denied.allowed, "Should be denied for secrets table");
}

#[test]
fn test_multiple_services_isolation() {
    let mut checker = PermissionChecker::new(false);
    
    let permission1 = ServicePermission {
        service_id: "service_a".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![PermissionAction::Get],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    let permission2 = ServicePermission {
        service_id: "service_b".to_string(),
        default_policy: "deny".to_string(),
        allowed_actions: vec![PermissionAction::Put],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission1);
    checker.add_service_permission(permission2);
    
    let context = PermissionContext {
        schema: "sys".to_string(),
        table: Some("test".to_string()),
        action: PermissionAction::Get,
    };
    
    let result_a = checker.check_permission("service_a", &context);
    assert!(result_a.allowed, "service_a should allow Get");
    
    let result_b = checker.check_permission("service_b", &context);
    assert!(!result_b.allowed, "service_b should deny Get (only allows Put)");
}

#[test]
fn test_list_services() {
    let mut checker = PermissionChecker::new(true);
    
    let permission1 = ServicePermission {
        service_id: "service_1".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    let permission2 = ServicePermission {
        service_id: "service_2".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission1);
    checker.add_service_permission(permission2);
    
    let services = checker.list_services();
    assert_eq!(services.len(), 2);
    assert!(services.contains(&"service_1".to_string()));
    assert!(services.contains(&"service_2".to_string()));
}

#[test]
fn test_get_service_policy() {
    let mut checker = PermissionChecker::new(true);
    
    let permission = ServicePermission {
        service_id: "specific_service".to_string(),
        default_policy: "deny".to_string(),
        allowed_actions: vec![PermissionAction::Get],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let found = checker.get_service_policy("specific_service");
    assert!(found.is_some());
    assert_eq!(found.unwrap().default_policy, "deny");
    
    let not_found = checker.get_service_policy("nonexistent_service");
    assert!(not_found.is_none());
}

#[test]
fn test_readonly_service_cannot_write() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "readonly".to_string(),
        default_policy: "deny".to_string(),
        allowed_actions: vec![
            PermissionAction::Get,
            PermissionAction::ListTables,
            PermissionAction::GetRow,
        ],
        denied_actions: vec![
            PermissionAction::Put,
            PermissionAction::Delete,
            PermissionAction::CreateTable,
            PermissionAction::DropTable,
        ],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let write_actions = vec![
        PermissionAction::Put,
        PermissionAction::Delete,
        PermissionAction::CreateTable,
    ];
    
    for action in write_actions {
        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: action.clone(),
        };
        
        let result = checker.check_permission("readonly", &context);
        assert!(!result.allowed, "Readonly service should deny write action");
    }
}

#[test]
fn test_writeonly_service_cannot_read() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "writeonly".to_string(),
        default_policy: "deny".to_string(),
        allowed_actions: vec![
            PermissionAction::Put,
            PermissionAction::Delete,
            PermissionAction::CreateTable,
        ],
        denied_actions: vec![
            PermissionAction::Get,
            PermissionAction::ListTables,
        ],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let read_actions = vec![
        PermissionAction::Get,
        PermissionAction::ListTables,
    ];
    
    for action in read_actions {
        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: action.clone(),
        };
        
        let result = checker.check_permission("writeonly", &context);
        assert!(!result.allowed, "Writeonly service should deny read action");
    }
}

#[test]
fn test_config_service_permission_lookup() {
    let yaml_config = r#"
db_path: ./test_db
default_policy: deny

access_protocols:
  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8080
    service_id: rest_admin

  - protocol: grpc
    enabled: true
    addr: 127.0.0.1:19777
    service_id: grpc_admin

permissions:
  - service_id: rest_admin
    default_policy: allow
    allowed_actions:
      - get
      - put
      - delete

  - service_id: grpc_admin
    default_policy: allow
    allowed_actions:
      - get
      - put
"#;
    
    let config: DatabaseConfig = serde_yaml::from_str(yaml_config).unwrap();
    
    let rest_perm = config.get_service_permission("rest_admin");
    assert!(rest_perm.is_some());
    assert_eq!(rest_perm.unwrap().default_policy, "allow");
    
    let grpc_perm = config.get_service_permission("grpc_admin");
    assert!(grpc_perm.is_some());
    assert_eq!(grpc_perm.unwrap().default_policy, "allow");
    
    let unknown_perm = config.get_service_permission("unknown");
    assert!(unknown_perm.is_none());
}

#[test]
fn test_config_global_default_policy() {
    let yaml_config = r#"
db_path: ./test_db
default_policy: allow
"#;
    
    let config: DatabaseConfig = serde_yaml::from_str(yaml_config).unwrap();
    assert!(config.get_global_default_policy());
    
    let yaml_config_deny = r#"
db_path: ./test_db
default_policy: deny
"#;
    
    let config_deny: DatabaseConfig = serde_yaml::from_str(yaml_config_deny).unwrap();
    assert!(!config_deny.get_global_default_policy());
}

#[test]
fn test_config_inline_permission() {
    let yaml_config = r#"
db_path: ./test_db

access_protocols:
  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8080
    service_id: inline_admin
    permissions:
      service_id: inline_admin
      default_policy: allow
      allowed_actions:
        - get
        - put
"#;
    
    let config: DatabaseConfig = serde_yaml::from_str(yaml_config).unwrap();
    
    let inline_perm = config.get_service_permission("inline_admin");
    assert!(inline_perm.is_some());
    assert_eq!(inline_perm.unwrap().default_policy, "allow");
}

#[test]
fn test_all_action_types() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "full_access".to_string(),
        default_policy: "deny".to_string(),
        allowed_actions: vec![
            PermissionAction::Get,
            PermissionAction::Put,
            PermissionAction::Delete,
            PermissionAction::CreateTable,
            PermissionAction::DropTable,
            PermissionAction::ListTables,
            PermissionAction::ListTableCols,
            PermissionAction::AddRow,
            PermissionAction::GetRow,
            PermissionAction::UpdateRow,
            PermissionAction::DeleteRow,
            PermissionAction::GetAllMeta,
            PermissionAction::GetSchemaInfo,
            PermissionAction::GetTableMeta,
        ],
        denied_actions: vec![],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let all_actions = vec![
        PermissionAction::Get,
        PermissionAction::Put,
        PermissionAction::Delete,
        PermissionAction::CreateTable,
        PermissionAction::DropTable,
        PermissionAction::ListTables,
        PermissionAction::ListTableCols,
        PermissionAction::AddRow,
        PermissionAction::GetRow,
        PermissionAction::UpdateRow,
        PermissionAction::DeleteRow,
        PermissionAction::GetAllMeta,
        PermissionAction::GetSchemaInfo,
        PermissionAction::GetTableMeta,
    ];
    
    for action in all_actions {
        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: action.clone(),
        };
        
        let result = checker.check_permission("full_access", &context);
        assert!(result.allowed, "Full access service should allow all actions");
    }
}

#[test]
fn test_service_id_consistency() {
    let yaml_config = r#"
db_path: ./test_db
default_policy: deny

access_protocols:
  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8080
    service_id: service_a
  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8081
    service_id: service_b

permissions:
  - service_id: service_a
    default_policy: allow
    allowed_actions:
      - get
  - service_id: service_b
    default_policy: deny
    allowed_actions:
      - put
"#;
    
    let config: DatabaseConfig = serde_yaml::from_str(yaml_config).unwrap();
    
    let services = config.get_service_ids();
    let perms = config.get_permission_service_ids();
    
    assert_eq!(services.len(), 2);
    assert_eq!(perms.len(), 2);
    
    for service_id in &services {
        let perm = config.get_service_permission(service_id);
        assert!(perm.is_some(), "Service {} should have corresponding permission", service_id);
    }
}

#[test]
fn test_permission_enforcement_order() {
    let mut checker = PermissionChecker::new(false);
    
    let permission = ServicePermission {
        service_id: "test".to_string(),
        default_policy: "allow".to_string(),
        allowed_actions: vec![],
        denied_actions: vec![PermissionAction::Delete],
        table_permissions: None,
    };
    
    checker.add_service_permission(permission);
    
    let context = PermissionContext {
        schema: "sys".to_string(),
        table: Some("test".to_string()),
        action: PermissionAction::Delete,
    };
    
    let result = checker.check_permission("test", &context);
    assert!(!result.allowed, "Explicit deny should take precedence over default allow");
}
