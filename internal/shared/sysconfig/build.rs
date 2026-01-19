use std::env;
use std::fs;
use std::path::Path;

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

fn generate_defaults(config: &toml::Value) -> String {
    let mut output = String::new();

    if let Some(system) = config.get("system") {
        output.push_str(&generate_impl_default("SystemConfig", system));
    }
    if let Some(network) = config.get("network") {
        output.push_str(&generate_impl_default("NetworkConfig", network));
    }
    if let Some(vm) = config.get("vm") {
        output.push_str(&generate_impl_default("VmConfig", vm));
    }

    output
}

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

fn toml_value_to_rust(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => format!("\"{}\".to_string()", s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) => {
            let elements: Vec<String> = arr.iter().map(|v| toml_value_to_rust(v)).collect();
            format!("vec![{}]", elements.join(", "))
        }
        _ => panic!("Unsupported TOML value type"),
    }
}
