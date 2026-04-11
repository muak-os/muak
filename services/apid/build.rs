//! Build script that parses proto files and generates RBAC rules.
//!
//! Proto files are the single source of truth for RBAC. Each RPC method must have
//! an `@rbac:` annotation comment on the line immediately preceding it.
//!
//! # Annotation Format
//!
//! ```protobuf
//! // @rbac: none                    // Unauthenticated: no cert needed ever, dangerous!
//! // @rbac: vm:read                 // Requires permission: always needs cert
//! // @rbac: maintenance|system:read // Maintenance mode: no cert; installed: needs permission
//! ```
//!
//! # Build Failures
//!
//! The build will fail if:
//! - Any RPC method lacks an `@rbac:` annotation
//! - A permission string is not recognized (e.g., `@rbac: invalid:perm`)

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Valid permission strings that map to Permission enum variants.
const VALID_PERMISSIONS: &[(&str, &str)] = &[
    ("admin", "Admin"),
    ("vm:read", "VmRead"),
    ("vm:create", "VmCreate"),
    ("vm:start", "VmStart"),
    ("vm:stop", "VmStop"),
    ("vm:delete", "VmDelete"),
    ("vm:upload", "VmUpload"),
    ("auth:manage", "AuthManage"),
    ("system:read", "SystemRead"),
    ("system:update", "SystemUpdate"),
    ("process:read", "ProcessRead"),
    ("security:read", "SecurityRead"),
];

/// Represents a parsed RPC method with its RBAC requirement.
#[derive(Debug)]
struct RpcMethod {
    full_path: String,
    requirement: RbacRequirement,
}

/// RBAC requirement for a method.
#[derive(Debug)]
enum RbacRequirement {
    Unauthenticated,
    RequiresPermission(String),
    MaintenanceOrPermission(String),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let out_dir = env::var("OUT_DIR")?;

    let proto_dir = Path::new(&manifest_dir).join("../../api");
    let proto_files = [
        "auth.proto",
        "vm.proto",
        "provision.proto",
        "process.proto",
        "security.proto",
        "log.proto",
        "version.proto",
    ];

    let mut all_methods = Vec::new();
    let mut errors = Vec::new();

    let perm_map: HashMap<&str, &str> = VALID_PERMISSIONS.iter().copied().collect();

    for proto_file in &proto_files {
        let proto_path = proto_dir.join(proto_file);
        println!("cargo:rerun-if-changed={}", proto_path.display());

        let content = match fs::read_to_string(&proto_path) {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("Failed to read {}: {}", proto_file, e));
                continue;
            }
        };

        match parse_proto(&content, proto_file, &perm_map) {
            Ok(methods) => all_methods.extend(methods),
            Err(errs) => errors.extend(errs),
        }
    }

    if !errors.is_empty() {
        for err in &errors {
            eprintln!("error: {}", err);
        }
        panic!(
            "Proto RBAC annotation errors found. See above for details.\n\
             All RPC methods must have an `// @rbac: permission` comment on the preceding line."
        );
    }

    let generated = generate_rust_code(&all_methods);
    let out_path = Path::new(&out_dir).join("rbac_rules.rs");
    let mut file = fs::File::create(&out_path)?;
    file.write_all(generated.as_bytes())?;
    Ok(())
}

/// Parses a proto file and extracts RPC methods with their RBAC annotations.
fn parse_proto(
    content: &str,
    filename: &str,
    perm_map: &HashMap<&str, &str>,
) -> Result<Vec<RpcMethod>, Vec<String>> {
    let mut methods = Vec::new();
    let mut errors = Vec::new();

    let lines: Vec<&str> = content.lines().collect();

    let package = lines
        .iter()
        .find_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("package ") {
                Some(
                    trimmed
                        .strip_prefix("package ")?
                        .trim_end_matches(';')
                        .trim(),
                )
            } else {
                None
            }
        })
        .unwrap_or("");

    let mut current_service: Option<&str> = None;
    let mut brace_depth = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("service ")
            && let Some(name_end) = rest.find(|c: char| c == '{' || c.is_whitespace())
        {
            current_service = Some(&rest[..name_end]);
        }

        brace_depth += trimmed.matches('{').count() as i32;
        brace_depth -= trimmed.matches('}').count() as i32;

        if brace_depth == 0 {
            current_service = None;
        }

        if trimmed.starts_with("rpc ") {
            let service = match current_service {
                Some(s) => s,
                None => {
                    errors.push(format!(
                        "{}:{}: RPC found outside of service block",
                        filename,
                        i + 1
                    ));
                    continue;
                }
            };

            let method_name = match extract_method_name(trimmed) {
                Some(name) => name,
                None => {
                    errors.push(format!(
                        "{}:{}: Could not parse RPC method name from: {}",
                        filename,
                        i + 1,
                        trimmed
                    ));
                    continue;
                }
            };

            let rbac_annotation = if i > 0 {
                extract_rbac_annotation(lines[i - 1])
            } else {
                None
            };

            let requirement = match rbac_annotation {
                Some(annotation) => match parse_rbac_annotation(&annotation, perm_map) {
                    Ok(req) => req,
                    Err(e) => {
                        errors.push(format!(
                            "{}:{}: Invalid @rbac annotation for {}: {}",
                            filename,
                            i + 1,
                            method_name,
                            e
                        ));
                        continue;
                    }
                },
                None => {
                    errors.push(format!(
                        "{}:{}: RPC method '{}' is missing @rbac annotation. \
                         Add `// @rbac: <permission>` on the line before the rpc declaration.",
                        filename,
                        i + 1,
                        method_name
                    ));
                    continue;
                }
            };

            let full_path = format!("/{}.{}/{}", package, service, method_name);
            methods.push(RpcMethod {
                full_path,
                requirement,
            });
        }
    }

    if errors.is_empty() {
        Ok(methods)
    } else {
        Err(errors)
    }
}

/// Extracts the method name from an RPC line like "rpc MethodName(...)"
fn extract_method_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("rpc ")?.trim();
    let end = rest.find('(')?;
    Some(rest[..end].trim())
}

/// Extracts the @rbac: annotation value from a comment line.
fn extract_rbac_annotation(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("//") {
        let rest = rest.trim();
        if let Some(annotation) = rest.strip_prefix("@rbac:") {
            return Some(annotation.trim().to_string());
        }
    }
    None
}

/// Parses an @rbac annotation value into an RbacRequirement.
fn parse_rbac_annotation(
    annotation: &str,
    perm_map: &HashMap<&str, &str>,
) -> Result<RbacRequirement, String> {
    let annotation = annotation.trim();

    if annotation == "none" {
        return Ok(RbacRequirement::Unauthenticated);
    }

    if let Some(perm_str) = annotation.strip_prefix("maintenance|") {
        let perm_str = perm_str.trim();
        return perm_map
            .get(perm_str)
            .map(|v| RbacRequirement::MaintenanceOrPermission((*v).to_string()))
            .ok_or_else(|| {
                format!(
                    "Unknown permission in maintenance annotation: '{}'",
                    perm_str
                )
            });
    }

    perm_map
        .get(annotation)
        .map(|v| RbacRequirement::RequiresPermission((*v).to_string()))
        .ok_or_else(|| format!("Unknown permission: '{}'", annotation))
}

/// Generates the Rust code for RBAC rules.
fn generate_rust_code(methods: &[RpcMethod]) -> String {
    let mut code = String::new();

    code.push_str(
        "// AUTO-GENERATED by build.rs - DO NOT EDIT\n\
         // Source of truth: api/*.proto files with @rbac: annotations\n\n\
         use config::Permission;\n\n",
    );

    code.push_str(
        "/// Result of looking up a method's permission requirement.\n\
         #[derive(Debug, Clone, PartialEq, Eq)]\n\
         pub enum MethodRequirement {\n\
         \x20   RequiresPermission(Permission),\n\
         \x20   MaintenanceOrPermission(Permission),\n\
         \x20   Unauthenticated,\n\
         \x20   Unknown,\n\
         }\n\n",
    );

    code.push_str(
        "/// Returns the permission requirement for a gRPC method path.\n\
         #[must_use]\n\
         pub fn method_permission(path: &str) -> MethodRequirement {\n\
         \x20   match path {\n",
    );

    for method in methods {
        let requirement_code = match &method.requirement {
            RbacRequirement::Unauthenticated => "MethodRequirement::Unauthenticated".to_string(),
            RbacRequirement::RequiresPermission(perm) => {
                format!(
                    "MethodRequirement::RequiresPermission(Permission::{})",
                    perm
                )
            }
            RbacRequirement::MaintenanceOrPermission(perm) => {
                format!(
                    "MethodRequirement::MaintenanceOrPermission(Permission::{})",
                    perm
                )
            }
        };
        code.push_str(&format!(
            "        \"{}\" => {},\n",
            method.full_path, requirement_code
        ));
    }

    code.push_str(
        "        _ => MethodRequirement::Unknown,\n\
         \x20   }\n\
         }\n\n",
    );

    code.push_str(
        "/// All known gRPC method paths.\n#[cfg(test)]\npub const KNOWN_METHODS: &[&str] = &[\n",
    );
    for method in methods {
        code.push_str(&format!("    \"{}\",\n", method.full_path));
    }
    code.push_str("];\n\n");

    code.push_str(
        "/// Methods that do not require authentication.\n\
         #[cfg(test)]\n\
         pub const UNAUTHENTICATED_METHODS: &[&str] = &[\n",
    );
    for method in methods {
        if matches!(method.requirement, RbacRequirement::Unauthenticated) {
            code.push_str(&format!("    \"{}\",\n", method.full_path));
        }
    }
    code.push_str("];\n\n");

    code.push_str(
        "/// Methods that require no auth in maintenance mode but need a permission when installed.\n\
         #[cfg(test)]\n\
         pub const MAINTENANCE_METHODS: &[&str] = &[\n",
    );
    for method in methods {
        if matches!(
            method.requirement,
            RbacRequirement::MaintenanceOrPermission(_)
        ) {
            code.push_str(&format!("    \"{}\",\n", method.full_path));
        }
    }
    code.push_str("];\n");

    code
}
