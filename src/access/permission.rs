use crate::config::{PermissionAction, ServicePermission};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PermissionContext {
    pub schema: String,
    pub table: Option<String>,
    pub action: PermissionAction,
}

#[derive(Debug, Clone)]
pub struct PermissionCheckResult {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Clone)]
pub struct PermissionChecker {
    global_default_allow: bool,
    service_permissions: HashMap<String, ServicePermission>,
}

impl PermissionChecker {
    pub fn new(global_default_allow: bool) -> Self {
        Self {
            global_default_allow,
            service_permissions: HashMap::new(),
        }
    }

    pub fn add_service_permission(&mut self, permission: ServicePermission) {
        self.service_permissions.insert(permission.service_id.clone(), permission);
    }

    pub fn check_permission(&self, service_id: &str, context: &PermissionContext) -> PermissionCheckResult {
        if let Some(permission) = self.service_permissions.get(service_id) {
            self.check_service_permission(permission, context)
        } else {
            PermissionCheckResult {
                allowed: self.global_default_allow,
                reason: if self.global_default_allow {
                    format!("Service '{}' has no specific permission, using global default (allow)", service_id)
                } else {
                    format!("Service '{}' has no specific permission, using global default (deny)", service_id)
                },
            }
        }
    }

    fn check_service_permission(&self, permission: &ServicePermission, context: &PermissionContext) -> PermissionCheckResult {
        if self.is_action_denied(permission, &context.action) {
            return PermissionCheckResult {
                allowed: false,
                reason: format!("Action '{}' is explicitly denied for service '{}'", context.action, permission.service_id),
            };
        }

        if self.is_action_allowed(permission, &context.action) {
            if let Some(ref table_perms) = permission.table_permissions {
                if !self.check_table_permission(table_perms, &context.schema, context.table.as_deref()) {
                    return PermissionCheckResult {
                        allowed: false,
                        reason: format!("Table/schema access denied for action '{}'", context.action),
                    };
                }
            }

            return PermissionCheckResult {
                allowed: true,
                reason: format!("Action '{}' is allowed for service '{}'", context.action, permission.service_id),
            };
        }

        let default_allow = permission.default_policy.to_lowercase() == "allow";
        PermissionCheckResult {
            allowed: default_allow,
            reason: format!(
                "Action '{}' not explicitly allowed/denied, using service default policy ({})",
                context.action,
                if default_allow { "allow" } else { "deny" }
            ),
        }
    }

    fn is_action_allowed(&self, permission: &ServicePermission, action: &PermissionAction) -> bool {
        permission.allowed_actions.iter().any(|a| a == action || a == &PermissionAction::All)
    }

    fn is_action_denied(&self, permission: &ServicePermission, action: &PermissionAction) -> bool {
        permission.denied_actions.iter().any(|a| a == action || a == &PermissionAction::All)
    }

    fn check_table_permission(&self, table_perms: &crate::config::TablePermissions, schema: &str, table: Option<&str>) -> bool {
        if !table_perms.allowed_schemas.is_empty() && !table_perms.allowed_schemas.iter().any(|s| s == schema) {
            return false;
        }

        if table_perms.denied_schemas.iter().any(|s| s == schema) {
            return false;
        }

        if let Some(table_name) = table {
            if !table_perms.allowed_tables.is_empty() && !table_perms.allowed_tables.iter().any(|t| t == table_name) {
                return false;
            }

            if table_perms.denied_tables.iter().any(|t| t == table_name) {
                return false;
            }
        }

        true
    }

    pub fn get_service_policy(&self, service_id: &str) -> Option<&ServicePermission> {
        self.service_permissions.get(service_id)
    }

    pub fn list_services(&self) -> Vec<String> {
        self.service_permissions.keys().cloned().collect()
    }
}

impl Default for PermissionChecker {
    fn default() -> Self {
        Self::new(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_allow_policy() {
        let checker = PermissionChecker::new(true);
        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };

        let result = checker.check_permission("unknown_service", &context);
        assert!(result.allowed);
    }

    #[test]
    fn test_default_deny_policy() {
        let checker = PermissionChecker::new(false);
        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };

        let result = checker.check_permission("unknown_service", &context);
        assert!(!result.allowed);
    }

    #[test]
    fn test_explicit_allow() {
        let mut checker = PermissionChecker::new(false);
        let permission = ServicePermission {
            service_id: "test_service".to_string(),
            default_policy: "deny".to_string(),
            allowed_actions: vec![PermissionAction::Get],
            denied_actions: vec![],
            table_permissions: None,
        };
        checker.add_service_permission(permission);

        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };

        let result = checker.check_permission("test_service", &context);
        assert!(result.allowed);
    }

    #[test]
    fn test_explicit_deny() {
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
            table: Some("test".to_string()),
            action: PermissionAction::Delete,
        };

        let result = checker.check_permission("test_service", &context);
        assert!(!result.allowed);
    }

    #[test]
    fn test_all_action_allow() {
        let mut checker = PermissionChecker::new(false);
        let permission = ServicePermission {
            service_id: "all_service".to_string(),
            default_policy: "deny".to_string(),
            allowed_actions: vec![PermissionAction::All],
            denied_actions: vec![],
            table_permissions: None,
        };
        checker.add_service_permission(permission);

        for action in &[
            PermissionAction::Get, PermissionAction::Put, PermissionAction::CreateTable,
            PermissionAction::Query
        ] {
            let context = PermissionContext {
                schema: "sys".to_string(),
                table: Some("test".to_string()),
                action: action.clone(),
            };
            let result = checker.check_permission("all_service", &context);
            assert!(result.allowed);
        }
    }

    #[test]
    fn test_all_action_deny() {
        let mut checker = PermissionChecker::new(true);
        let permission = ServicePermission {
            service_id: "deny_all".to_string(),
            default_policy: "allow".to_string(),
            allowed_actions: vec![],
            denied_actions: vec![PermissionAction::All],
            table_permissions: None,
        };
        checker.add_service_permission(permission);

        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };
        let result = checker.check_permission("deny_all", &context);
        assert!(!result.allowed);
    }

    #[test]
    fn test_table_permission_allowed_schemas() {
        let mut checker = PermissionChecker::new(false);
        let permission = ServicePermission {
            service_id: "table_service".to_string(),
            default_policy: "allow".to_string(),
            allowed_actions: vec![PermissionAction::Get],
            denied_actions: vec![],
            table_permissions: Some(crate::config::TablePermissions {
                allowed_schemas: vec!["sys".to_string()],
                denied_schemas: vec![],
                allowed_tables: vec![],
                denied_tables: vec![],
            }),
        };
        checker.add_service_permission(permission);

        let allowed_context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };
        assert!(checker.check_permission("table_service", &allowed_context).allowed);

        let denied_context = PermissionContext {
            schema: "other".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };
        assert!(!checker.check_permission("table_service", &denied_context).allowed);
    }

    #[test]
    fn test_table_permission_denied_schemas() {
        let mut checker = PermissionChecker::new(true);
        let permission = ServicePermission {
            service_id: "table_service".to_string(),
            default_policy: "allow".to_string(),
            allowed_actions: vec![PermissionAction::Get],
            denied_actions: vec![],
            table_permissions: Some(crate::config::TablePermissions {
                allowed_schemas: vec![],
                denied_schemas: vec!["private".to_string()],
                allowed_tables: vec![],
                denied_tables: vec![],
            }),
        };
        checker.add_service_permission(permission);

        let denied_context = PermissionContext {
            schema: "private".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };
        assert!(!checker.check_permission("table_service", &denied_context).allowed);
    }

    #[test]
    fn test_list_services() {
        let mut checker = PermissionChecker::new(true);
        checker.add_service_permission(ServicePermission {
            service_id: "service1".to_string(),
            default_policy: "allow".to_string(),
            allowed_actions: vec![],
            denied_actions: vec![],
            table_permissions: None,
        });
        checker.add_service_permission(ServicePermission {
            service_id: "service2".to_string(),
            default_policy: "allow".to_string(),
            allowed_actions: vec![],
            denied_actions: vec![],
            table_permissions: None,
        });
        let services = checker.list_services();
        assert_eq!(services.len(), 2);
        assert!(services.contains(&"service1".to_string()));
        assert!(services.contains(&"service2".to_string()));
    }

    #[test]
    fn test_default_for_permission_checker() {
        let checker = PermissionChecker::default();
        let context = PermissionContext {
            schema: "sys".to_string(),
            table: Some("test".to_string()),
            action: PermissionAction::Get,
        };
        let result = checker.check_permission("unknown", &context);
        assert!(result.allowed);
    }
}
