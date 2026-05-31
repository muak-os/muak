//! Procedural macros for the granola service framework.

use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, ItemFn, LitStr, Pat, ReturnType, parse_macro_input};

/// Wraps a service entry point with lifecycle boilerplate.
#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name = parse_macro_input!(attr as LitStr);
    let input = parse_macro_input!(item as ItemFn);

    match generate(&name, input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Generates the wrapper code for a service entry point.
fn generate(name: &LitStr, mut input: ItemFn) -> Result<proc_macro2::TokenStream, syn::Error> {
    let notifier_ident = extract_notifier_param(&mut input)?;
    validate_return_type(&input)?;

    let attrs = &input.attrs;
    let body = &input.block;
    let service_name = &name;
    let start_msg = format!("Starting {}", name.value());
    let is_async = input.sig.asyncness.is_some();

    if is_async {
        Ok(generate_async(
            attrs,
            service_name,
            &start_msg,
            &notifier_ident,
            body,
        ))
    } else {
        Ok(generate_sync(
            attrs,
            service_name,
            &start_msg,
            &notifier_ident,
            body,
        ))
    }
}

/// Generates the async wrapper variant.
fn generate_async(
    attrs: &[syn::Attribute],
    service_name: &LitStr,
    start_msg: &str,
    notifier_ident: &syn::Ident,
    body: &syn::Block,
) -> proc_macro2::TokenStream {
    quote! {
        #(#attrs)*
        async fn main() {
            if let Err(e) = __granola_service_main().await {
                ::granola::kmsg::error!(@ #service_name, "Fatal error: {:#}", e);
                ::std::process::exit(1);
            }
        }

        async fn __granola_service_main() -> ::anyhow::Result<()> {
            ::granola::kmsg::init(#service_name)?;
            ::granola::kmsg::info!(#start_msg);
            let #notifier_ident = ::granola::NotifyClient::new(#service_name)?;
            let __result: ::anyhow::Result<()> = async { #body }.await;
            let _ = #notifier_ident.stopping("Graceful shutdown");
            __result
        }
    }
}

/// Generates the sync wrapper variant.
fn generate_sync(
    attrs: &[syn::Attribute],
    service_name: &LitStr,
    start_msg: &str,
    notifier_ident: &syn::Ident,
    body: &syn::Block,
) -> proc_macro2::TokenStream {
    quote! {
        #(#attrs)*
        fn main() {
            if let Err(e) = __granola_service_main() {
                ::granola::kmsg::error!(@ #service_name, "Fatal error: {:#}", e);
                ::std::process::exit(1);
            }
        }

        fn __granola_service_main() -> ::anyhow::Result<()> {
            ::granola::kmsg::init(#service_name)?;
            ::granola::kmsg::info!(#start_msg);
            let #notifier_ident = ::granola::NotifyClient::new(#service_name)?;
            let __result: ::anyhow::Result<()> = (|| { #body })();
            let _ = #notifier_ident.stopping("Graceful shutdown");
            __result
        }
    }
}

/// Extracts and removes the `NotifyClient` parameter from the function signature.
fn extract_notifier_param(input: &mut ItemFn) -> Result<syn::Ident, syn::Error> {
    if input.sig.inputs.len() != 1 {
        return Err(syn::Error::new_spanned(
            &input.sig,
            "#[granola::service] requires exactly one parameter: notifier: NotifyClient",
        ));
    }

    let arg = input
        .sig
        .inputs
        .pop()
        .ok_or_else(|| syn::Error::new_spanned(&input.sig, "expected exactly one parameter"))?
        .into_value();
    let FnArg::Typed(pat_type) = arg else {
        return Err(syn::Error::new_spanned(arg, "expected a typed parameter"));
    };

    let Pat::Ident(pat_ident) = *pat_type.pat else {
        return Err(syn::Error::new_spanned(
            pat_type.pat,
            "expected a simple identifier pattern",
        ));
    };

    Ok(pat_ident.ident)
}

/// Validates that the function returns `Result<()>`.
fn validate_return_type(input: &ItemFn) -> Result<(), syn::Error> {
    if matches!(input.sig.output, ReturnType::Default) {
        return Err(syn::Error::new_spanned(
            &input.sig,
            "#[granola::service] function must return Result<()>",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_notifier_from_valid_signature() {
        // ARRANGE
        let mut func: ItemFn =
            syn::parse_str("async fn main(notifier: NotifyClient) -> Result<()> { Ok(()) }")
                .expect("parse");

        // ACT
        let ident = extract_notifier_param(&mut func).expect("extract");

        // ASSERT
        assert_eq!(ident, "notifier");
        assert!(func.sig.inputs.is_empty());
    }

    #[test]
    fn extract_notifier_rejects_no_params() {
        // ARRANGE
        let mut func: ItemFn = syn::parse_str("fn main() -> Result<()> { Ok(()) }").expect("parse");

        // ACT
        let err = extract_notifier_param(&mut func);

        // ASSERT
        err.unwrap_err();
    }

    #[test]
    fn extract_notifier_rejects_multiple_params() {
        // ARRANGE
        let mut func: ItemFn =
            syn::parse_str("fn main(a: A, b: B) -> Result<()> { Ok(()) }").expect("parse");

        // ACT
        let err = extract_notifier_param(&mut func);

        // ASSERT
        err.unwrap_err();
    }

    #[test]
    fn validate_return_type_rejects_unit() {
        // ARRANGE
        let func: ItemFn = syn::parse_str("fn main(n: N) {}").expect("parse");

        // ACT
        let err = validate_return_type(&func);

        // ASSERT
        assert!(err.is_err());
    }

    #[test]
    fn validate_return_type_accepts_result() {
        // ARRANGE
        let func: ItemFn = syn::parse_str("fn main(n: N) -> Result<()> { Ok(()) }").expect("parse");

        // ACT
        let result = validate_return_type(&func);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn generate_produces_async_wrapper() {
        // ARRANGE
        let name: LitStr = syn::parse_str("\"testd\"").expect("parse");
        let func: ItemFn =
            syn::parse_str("async fn main(notifier: NotifyClient) -> Result<()> { Ok(()) }")
                .expect("parse");

        // ACT
        let output = generate(&name, func).expect("generate");
        let output_str = output.to_string();

        // ASSERT
        assert!(output_str.contains("__granola_service_main"));
        assert!(output_str.contains(". await"));
        assert!(output_str.contains("\"testd\""));
        assert!(output_str.contains(":: granola"));
    }

    #[test]
    fn generate_produces_sync_wrapper() {
        // ARRANGE
        let name: LitStr = syn::parse_str("\"modd\"").expect("parse");
        let func: ItemFn =
            syn::parse_str("fn main(notifier: NotifyClient) -> Result<()> { Ok(()) }")
                .expect("parse");

        // ACT
        let output = generate(&name, func).expect("generate");
        let output_str = output.to_string();

        // ASSERT
        assert!(output_str.contains("__granola_service_main"));
        assert!(!output_str.contains(". await"));
        assert!(output_str.contains("\"modd\""));
    }

    #[test]
    fn generate_preserves_custom_notifier_name() {
        // ARRANGE
        let name: LitStr = syn::parse_str("\"svc\"").expect("parse");
        let func: ItemFn =
            syn::parse_str("fn main(ctx: NotifyClient) -> Result<()> { Ok(()) }").expect("parse");

        // ACT
        let output = generate(&name, func).expect("generate");
        let output_str = output.to_string();

        // ASSERT
        assert!(output_str.contains("let ctx"));
    }

    #[test]
    fn generate_async_contains_start_message() {
        // ARRANGE
        let name: LitStr = syn::parse_str("\"myservice\"").expect("parse");
        let func: ItemFn =
            syn::parse_str("async fn main(n: NotifyClient) -> Result<()> { Ok(()) }")
                .expect("parse");

        // ACT
        let output = generate(&name, func).expect("generate");
        let output_str = output.to_string();

        // ASSERT
        assert!(output_str.contains("Starting myservice"));
    }

    #[test]
    fn generate_sync_contains_stopping_call() {
        // ARRANGE
        let name: LitStr = syn::parse_str("\"svc\"").expect("parse");
        let func: ItemFn =
            syn::parse_str("fn main(n: NotifyClient) -> Result<()> { Ok(()) }").expect("parse");

        // ACT
        let output = generate(&name, func).expect("generate");
        let output_str = output.to_string();

        // ASSERT
        assert!(output_str.contains("stopping"));
        assert!(output_str.contains("Graceful shutdown"));
    }
}
