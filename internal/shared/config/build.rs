use std::env;
use std::fs;
use std::path::Path;

/// Build script entry point.
/// Generates default implementations for config structs based on default.toml.
fn main() {
    let default_toml_path = Path::new("../../default.toml");
    let default_toml = fs::read_to_string(default_toml_path).expect("Failed to read default.toml");
    let config: toml::Value = toml::from_str(&default_toml).expect("Failed to parse default.toml");

    let generated_code = generate_defaults(&config);

    let out_dir = env::var("OUT_DIR").unwrap();
    let defaults_rs_path = Path::new(&out_dir).join("defaults.rs");
    fs::write(&defaults_rs_path, generated_code).expect("Failed to write defaults.rs");

    println!("cargo:rerun-if-changed=../../default.toml");
}

/// Generates Default impls for config structs from the parsed TOML config.
fn generate_defaults(config: &toml::Value) -> String {
    let mut output = String::new();

    if let Some(host) = config.get("host") {
        output.push_str(&generate_impl_default("HostConfig", host));
    }
    if let Some(network) = config.get("network") {
        output.push_str(&generate_network_default(network));
    }
    if let Some(vm) = config.get("vm") {
        output.push_str(&generate_impl_default("VmConfig", vm));
    }

    output
}

/// Generates a Default impl for a specific struct from its TOML table.
fn generate_impl_default(struct_name: &str, table: &toml::Value) -> String {
    let mut fields = Vec::new();
    if let Some(table) = table.as_table() {
        for (key, value) in table {
            let rust_value = toml_value_to_rust(value);
            fields.push(format!("            {}: {},", key, rust_value));
        }
    }
    format!(
        "impl Default for {} {{
    fn default() -> Self {{
        Self {{
{}
        }}
    }}
}}

",
        struct_name,
        fields.join("\n")
    )
}

/// Generates a Default impl for NetworkConfig, handling the interfaces array-of-tables.
fn generate_network_default(network: &toml::Value) -> String {
    let mut fields = Vec::new();
    if let Some(table) = network.as_table() {
        for (key, value) in table {
            if key == "interfaces" {
                fields.push(format!(
                    "            interfaces: {},",
                    toml_interfaces_to_rust(value)
                ));
            } else {
                let rust_value = toml_value_to_rust(value);
                fields.push(format!("            {}: {},", key, rust_value));
            }
        }
    }
    format!(
        "impl Default for NetworkConfig {{
    fn default() -> Self {{
        Self {{
{}
        }}
    }}
}}

",
        fields.join("\n")
    )
}

/// Converts the `[[network.interfaces]]` array-of-tables into Rust `vec![...]` literal.
fn toml_interfaces_to_rust(value: &toml::Value) -> String {
    let Some(arr) = value.as_array() else {
        return "vec![]".to_string();
    };

    if arr.is_empty() {
        return "vec![]".to_string();
    }

    let items: Vec<String> = arr.iter().map(toml_interface_to_rust).collect();
    format!("vec![{}]", items.join(", "))
}

/// Converts a single interface table entry to an `InterfaceConfig { ... }` literal.
fn toml_interface_to_rust(value: &toml::Value) -> String {
    let Some(table) = value.as_table() else {
        panic!("interfaces entry must be a table");
    };

    let name = table
        .get("name")
        .and_then(|v| v.as_str())
        .expect("interface must have a name");

    let kind = table
        .get("type")
        .and_then(|v| v.as_str())
        .expect("interface must have a type");

    let kind_variant = match kind {
        "bridge" => "crate::system::InterfaceKind::Bridge",
        "ethernet" => "crate::system::InterfaceKind::Ethernet",
        other => panic!("unknown interface type: {}", other),
    };

    let ipv4 = table.get("ipv4").map(toml_ipv4_to_rust);
    let ipv6 = table.get("ipv6").map(toml_ipv6_to_rust);
    let bridge = table.get("bridge").map(toml_bridge_to_rust);

    format!(
        "crate::system::InterfaceConfig {{ name: \"{name}\".to_string(), kind: {kind_variant}, ipv4: {ipv4}, ipv6: {ipv6}, bridge: {bridge} }}",
        name = name,
        kind_variant = kind_variant,
        ipv4 = ipv4
            .map(|s| format!("Some({})", s))
            .unwrap_or_else(|| "None".to_string()),
        ipv6 = ipv6
            .map(|s| format!("Some({})", s))
            .unwrap_or_else(|| "None".to_string()),
        bridge = bridge
            .map(|s| format!("Some({})", s))
            .unwrap_or_else(|| "None".to_string()),
    )
}

fn toml_ipv4_to_rust(value: &toml::Value) -> String {
    let dhcp = value.get("dhcp").and_then(|v| v.as_bool()).unwrap_or(false);
    format!(
        "crate::system::Ipv4InterfaceConfig {{ dhcp: {dhcp}, address: None, prefix: None, gateway: None }}",
        dhcp = dhcp,
    )
}

fn toml_ipv6_to_rust(value: &toml::Value) -> String {
    let autoconf = value
        .get("autoconf")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    format!(
        "crate::system::Ipv6InterfaceConfig {{ autoconf: {autoconf} }}",
        autoconf = autoconf,
    )
}

fn toml_bridge_to_rust(value: &toml::Value) -> String {
    let stp = value.get("stp").and_then(|v| v.as_bool()).unwrap_or(false);
    let port = value
        .get("port")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let items: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| format!("\"{}\".to_string()", s))
                .collect();
            format!("vec![{}]", items.join(", "))
        })
        .unwrap_or_else(|| "vec![]".to_string());
    format!(
        "crate::system::BridgeConfig {{ port: {port}, stp: {stp} }}",
        port = port,
        stp = stp,
    )
}

/// Converts a TOML value to Rust literal syntax.
/// Supports strings, integers, booleans, and flat arrays of primitives.
fn toml_value_to_rust(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => format!("\"{}\".to_string()", s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) => {
            let elements: Vec<String> = arr.iter().map(toml_value_to_rust).collect();
            format!("vec![{}]", elements.join(", "))
        }
        _ => panic!("Unsupported TOML value type in toml_value_to_rust"),
    }
}
