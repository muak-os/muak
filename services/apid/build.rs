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

extern crate alloc;

use alloc::collections::BTreeMap;
use core::error::Error;
use core::fmt::Write as _;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write as _;
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

fn main() -> Result<(), Box<dyn Error>> {
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
            Ok(content) => content,
            Err(e) => {
                errors.push(format!("Failed to read {proto_file}: {e}"));
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
            eprintln!("error: {err}");
        }
        return Err(
            "Proto RBAC annotation errors found. See above for details. \
             All RPC methods must have an `// @rbac: permission` comment on the preceding line."
                .into(),
        );
    }

    let generated = generate_rust_code(&all_methods)?;
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
    let mut parser = ProtoParser {
        lines: &content.lines().collect::<Vec<_>>(),
        package: extract_package(content),
        filename,
        perm_map,
        methods: Vec::new(),
        errors: Vec::new(),
    };

    let lines = parser.lines;

    let mut current_service: Option<&str> = None;
    let mut brace_depth: usize = 0;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("service ")
            && let Some(name_end) = rest.find(|ch: char| ch == '{' || ch.is_whitespace())
        {
            current_service = Some(rest.get(..name_end).unwrap_or_default());
        }

        brace_depth = brace_depth.saturating_add(trimmed.matches('{').count());
        brace_depth = brace_depth.saturating_sub(trimmed.matches('}').count());

        if brace_depth == 0 {
            current_service = None;
        }

        if trimmed.starts_with("rpc ") {
            parse_rpc_line(&mut parser, trimmed, idx, current_service);
        }
    }

    if parser.errors.is_empty() {
        Ok(parser.methods)
    } else {
        Err(parser.errors)
    }
}

/// Extracts the proto package name, if any.
fn extract_package(content: &str) -> &str {
    content
        .lines()
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
        .unwrap_or("")
}

/// Shared state while parsing a proto file.
struct ProtoParser<'a> {
    lines: &'a [&'a str],
    package: &'a str,
    filename: &'a str,
    perm_map: &'a HashMap<&'a str, &'a str>,
    methods: Vec<RpcMethod>,
    errors: Vec<String>,
}

/// Parses a single `rpc` line and appends the resulting method or error.
fn parse_rpc_line(
    parser: &mut ProtoParser<'_>,
    trimmed: &str,
    idx: usize,
    current_service: Option<&str>,
) {
    let filename = parser.filename;
    let package = parser.package;
    let line_num = idx.saturating_add(1);

    let Some(service) = current_service else {
        parser.errors.push(format!(
            "{filename}:{line_num}: RPC found outside of service block"
        ));
        return;
    };

    let Some(method_name) = extract_method_name(trimmed) else {
        parser.errors.push(format!(
            "{filename}:{line_num}: Could not parse RPC method name from: {trimmed}"
        ));
        return;
    };

    let rbac_annotation = if idx > 0 {
        parser
            .lines
            .get(idx.saturating_sub(1))
            .and_then(|&line| extract_rbac_annotation(line))
    } else {
        None
    };

    let Some(annotation) = rbac_annotation else {
        parser.errors.push(format!(
            "{filename}:{line_num}: RPC method '{method_name}' is missing @rbac annotation. \
             Add `// @rbac: <permission>` on the line before the rpc declaration."
        ));
        return;
    };

    let requirement = match parse_rbac_annotation(&annotation, parser.perm_map) {
        Ok(req) => req,
        Err(e) => {
            parser.errors.push(format!(
                "{filename}:{line_num}: Invalid @rbac annotation for {method_name}: {e}"
            ));
            return;
        }
    };

    let full_path = format!("/{package}.{service}/{method_name}");
    parser.methods.push(RpcMethod {
        full_path,
        requirement,
    });
}

/// Extracts the method name from an RPC line like `rpc MethodName(...)`.
fn extract_method_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("rpc ")?.trim();
    let end = rest.find('(')?;
    Some(rest.get(..end).unwrap_or_default().trim())
}

/// Extracts the `@rbac:` annotation value from a comment line.
fn extract_rbac_annotation(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("//") {
        let rest = rest.trim();
        if let Some(annotation) = rest.strip_prefix("@rbac:") {
            return Some(annotation.trim().to_owned());
        }
    }
    None
}

/// Parses an `@rbac` annotation value into an `RbacRequirement`.
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
            .map(|variant| RbacRequirement::MaintenanceOrPermission((*variant).to_owned()))
            .ok_or_else(|| format!("Unknown permission in maintenance annotation: '{perm_str}'"));
    }

    perm_map
        .get(annotation)
        .map(|variant| RbacRequirement::RequiresPermission((*variant).to_owned()))
        .ok_or_else(|| format!("Unknown permission: '{annotation}'"))
}

/// Generates the Rust code for RBAC rules.
fn generate_rust_code(methods: &[RpcMethod]) -> Result<String, core::fmt::Error> {
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

    // Group paths by their requirement so each match arm body is unique.
    let mut requirement_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for method in methods {
        let requirement_code = match method.requirement {
            RbacRequirement::Unauthenticated => "MethodRequirement::Unauthenticated".to_owned(),
            RbacRequirement::RequiresPermission(ref perm) => {
                format!("MethodRequirement::RequiresPermission(Permission::{perm})")
            }
            RbacRequirement::MaintenanceOrPermission(ref perm) => {
                format!("MethodRequirement::MaintenanceOrPermission(Permission::{perm})")
            }
        };
        requirement_groups
            .entry(requirement_code)
            .or_default()
            .push(method.full_path.clone());
    }

    for (requirement_code, paths) in requirement_groups {
        let patterns = paths
            .iter()
            .map(|path| format!("\"{path}\""))
            .collect::<Vec<_>>()
            .join(" | ");
        writeln!(code, "        {patterns} => {requirement_code},")?;
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
        let method_full_path = &method.full_path;
        writeln!(code, "    \"{method_full_path}\",")?;
    }
    code.push_str("];\n\n");

    code.push_str(
        "/// Methods that do not require authentication.\n\
         #[cfg(test)]\n\
         pub const UNAUTHENTICATED_METHODS: &[&str] = &[\n",
    );
    for method in methods {
        if matches!(method.requirement, RbacRequirement::Unauthenticated) {
            let method_full_path = &method.full_path;
            writeln!(code, "    \"{method_full_path}\",")?;
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
            let method_full_path = &method.full_path;
            writeln!(code, "    \"{method_full_path}\",")?;
        }
    }
    code.push_str("];\n");

    Ok(code)
}
