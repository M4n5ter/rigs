//! Fork from https://docs.rs/crate/rig-tool-macro/0.5.0

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Ident, LitStr, Meta, Result, Token};
use syn::{FnArg, ItemFn, PatType, ReturnType, Type, parse_macro_input};

#[derive(Debug, Default)]
struct ToolAttribute {
    name: Option<String>,
    description: Option<String>,
    args: Vec<ArgMeta>,
}

#[derive(Debug)]
struct ArgMeta {
    name: String,
    description: Option<String>,
}

impl Parse for ToolAttribute {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut attr = ToolAttribute::default();

        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

        for meta in metas {
            match meta {
                Meta::NameValue(nv) => {
                    let ident = nv
                        .path
                        .get_ident()
                        .ok_or_else(|| Error::new_spanned(&nv.path, "Expected identifier"))?;

                    let value = nv.value.clone();
                    let lit_result = syn::parse2::<LitStr>(nv.value.into_token_stream());
                    match (ident.to_string().as_str(), lit_result) {
                        ("name", Ok(lit)) => attr.name = Some(lit.value()),
                        ("description", Ok(lit)) => attr.description = Some(lit.value()),
                        (_, Err(e)) => {
                            return Err(Error::new_spanned(
                                value,
                                format!("Expected string literal, error: {e}"),
                            ));
                        }
                        _ => {
                            return Err(Error::new_spanned(
                                ident,
                                format!("Unknown attribute key: {}", ident),
                            ));
                        }
                    }
                }

                Meta::List(list) if list.path.is_ident("arg") => {
                    let args =
                        list.parse_args_with(Punctuated::<ArgMeta, Token![,]>::parse_terminated)?;
                    attr.args.append(&mut args.into_iter().collect());
                }

                meta => {
                    return Err(Error::new_spanned(
                        meta,
                        "Unsupported attribute format, expected `key = value` or `arg(...)`",
                    ));
                }
            }
        }

        Ok(attr)
    }
}

impl Parse for ArgMeta {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut arg = ArgMeta {
            name: input.parse::<Ident>()?.to_string().trim().to_owned(),
            description: None,
        };

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

            for meta in metas {
                match meta {
                    Meta::NameValue(nv) => {
                        let ident = nv
                            .path
                            .get_ident()
                            .ok_or_else(|| Error::new_spanned(&nv.path, "Expected identifier"))?;

                        let value = nv.value.clone();
                        match ident.to_string().as_str() {
                            "description" => {
                                let lit = syn::parse2::<LitStr>(nv.value.into_token_stream())
                                    .map_err(|e| {
                                        Error::new_spanned(
                                            value,
                                            format!("Expected string literal for description, error: {}", e),
                                        )
                                    })?;
                                arg.description = Some(lit.value());
                            }
                            _ => {
                                return Err(Error::new_spanned(
                                    ident,
                                    format!("Unknown arg property: {}", ident),
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(Error::new_spanned(
                            meta,
                            "Expected `key = value` format for arg properties",
                        ));
                    }
                }
            }
        }

        Ok(arg)
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

fn get_json_type(ty: &Type) -> TokenStream2 {
    match ty {
        Type::Path(type_path) => {
            let segment = &type_path.path.segments[0];
            let type_name = segment.ident.to_string();

            // Handle Vec types
            if type_name == "Vec" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let syn::GenericArgument::Type(inner_type) = &args.args[0] {
                        let inner_json_type = get_json_type(inner_type);
                        return quote! {
                            "type": "array",
                            "items": { #inner_json_type }
                        };
                    }
                }
                return quote! { "type": "array" };
            }

            // Handle primitive types
            match type_name.as_str() {
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" => {
                    quote! { "type": "number" }
                }
                "String" | "str" => {
                    quote! { "type": "string" }
                }
                "bool" => {
                    quote! { "type": "boolean" }
                }
                // Handle other types as objects
                _ => {
                    quote! { "type": "object" }
                }
            }
        }
        _ => quote! { "type": "object" },
    }
}

/// Check if the given type is a custom struct or Vec<Struct> (not a primitive or standard library type)
fn is_custom_struct(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => {
            let segment = &type_path.path.segments[0];
            let type_name = segment.ident.to_string();

            // Check if it's a Vec<T>
            if type_name == "Vec" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let syn::GenericArgument::Type(inner_type) = &args.args[0] {
                        return is_custom_struct(inner_type);
                    }
                }
                return false;
            }

            // List of known primitive and standard library types
            !matches!(
                type_name.as_str(),
                "i8" | "i16"
                    | "i32"
                    | "i64"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "usize"
                    | "f32"
                    | "f64"
                    | "bool"
                    | "String"
                    | "str"
                    | "Vec"
                    | "Option"
                    | "Result"
            )
        }
        _ => false,
    }
}

pub fn tool_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let tool_attr = parse_macro_input!(attr as ToolAttribute);
    let input_fn = parse_macro_input!(item as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let tool_name = match tool_attr.name {
        Some(name) => name,
        None => input_fn.sig.ident.to_string(),
    };

    let struct_name = quote::format_ident!("{}Tool", to_pascal_case(&tool_name));
    let static_name = quote::format_ident!("{}", to_pascal_case(&tool_name));

    // Extract return type: Result<T, E>
    let (return_type, error_type) = if let ReturnType::Type(_, ty) = &input_fn.sig.output {
        if let Type::Path(type_path) = ty.as_ref() {
            if type_path.path.segments[0].ident == "Result" {
                match &type_path.path.segments[0].arguments {
                    syn::PathArguments::AngleBracketed(args) => {
                        let params: Vec<_> = args.args.iter().collect();

                        if params.is_empty() || params.len() > 2 {
                            panic!("Result must have 1 or 2 type parameters");
                        }

                        let t = match params[0] {
                            syn::GenericArgument::Type(ty) => ty,
                            _ => panic!("Result must have a type parameter"),
                        };

                        let e = if params.len() == 2 {
                            match params[1] {
                                syn::GenericArgument::Type(ty) => ty.clone(),
                                _ => panic!("Result must have a type parameter"),
                            }
                        } else {
                            panic!("Result must have a type parameter");
                        };

                        (t, e)
                    }
                    _ => panic!("Result must have type parameters"),
                }
            } else {
                panic!("Function must return a Result<T, E> or Result<T>")
            }
        } else {
            panic!("Expected angle bracketed arguments in Result")
        }
    } else {
        panic!("Function must return a Result")
    };

    let args = input_fn.sig.inputs.iter().filter_map(|arg| {
        if let FnArg::Typed(PatType { pat, ty, .. }) = arg {
            Some((pat, ty))
        } else {
            None
        }
    });

    let arg_names: Vec<_> = args.clone().map(|(pat, _)| pat).collect();
    let arg_types: Vec<_> = args.clone().map(|(_, ty)| ty).collect();
    let json_types: Vec<_> = arg_types.iter().map(|ty| get_json_type(ty)).collect();

    let is_struct_args = arg_types.iter().any(|ty| is_custom_struct(ty));

    // if arg is struct,  it should be the only arg
    if is_struct_args && arg_types.len() != 1 {
        panic!("Struct args must be the only arg");
    }

    // arg attributes must be one of the function arguments
    for arg in &tool_attr.args {
        if !arg_names.iter().any(|pat| {
            if let syn::Pat::Ident(pat_ident) = &***pat {
                pat_ident.ident == arg.name
            } else {
                false
            }
        }) {
            panic!("Argument {} not found in function arguments", arg.name);
        }
    }

    // arg attributes must have a description
    for arg in &tool_attr.args {
        if arg.description.is_none() {
            panic!("Argument {} must have a description", arg.name);
        }
    }

    // an arg can not appear more than once, otherwise will panic
    let mut arg_names_set = std::collections::HashSet::new();
    for arg in &tool_attr.args {
        if arg_names_set.contains(&arg.name) {
            panic!("Argument {} appears more than once", arg.name);
        }
        arg_names_set.insert(arg.name.clone());
    }

    let arg_descriptions: Vec<_> = arg_names
        .iter()
        .map(|pat| {
            let ident = match &***pat {
                syn::Pat::Ident(pat_ident) => &pat_ident.ident,
                _ => panic!("Only simple identifiers are supported in tool arguments"),
            };
            let arg_meta = tool_attr.args.iter().find(|arg| *ident == arg.name);
            arg_meta
                .and_then(|arg| arg.description.clone())
                .unwrap_or_else(|| format!("Parameter {}", ident))
        })
        .collect();

    let args_struct_name = quote::format_ident!("{}Args", to_pascal_case(&tool_name));

    let call_impl = if input_fn.sig.asyncness.is_some() {
        quote! {
            async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
                #fn_name(#(args.#arg_names),*).await
            }
        }
    } else {
        quote! {
            async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
                #fn_name(#(args.#arg_names),*)
            }
        }
    };
    // Modify the definition implementation to use the description
    let description = match tool_attr.description {
        Some(desc) => quote! { #desc.to_string() },
        None => quote! { format!("Function to {}", Self::NAME) },
    };

    let definition_impl = if !is_struct_args {
        quote! {
            async fn definition(&self, _prompt: String) -> rigs::rig::completion::ToolDefinition {
                rigs::rig::completion::ToolDefinition {
                    name: Self::NAME.to_string(),
                    description: #description,
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            #(
                                stringify!(#arg_names): {
                                    #json_types,
                                    "description": #arg_descriptions
                                }
                            ),*
                        },
                    }),
                }
            }
        }
    } else {
        quote! {
            async fn definition(&self, _prompt: String) -> rigs::rig::completion::ToolDefinition {
                rigs::rig::completion::ToolDefinition {
                    name: Self::NAME.to_string(),
                    description: #description,
                    parameters: schemars::schema_for!(#args_struct_name).as_value().to_owned(),
                }
            }
        }
    };

    let expanded = quote! {
        #[allow(missing_docs)]
        #[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
        pub struct #struct_name;

        #[allow(missing_docs)]
        #[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
        pub struct #args_struct_name {
            #(#arg_names: #arg_types),*
        }

        #input_fn

        impl rigs::rig::tool::Tool for #struct_name {
            const NAME: &'static str = #tool_name;

            type Error = #error_type;
            type Args = #args_struct_name;
            type Output = #return_type;

            #definition_impl

            #call_impl
        }

        #[allow(missing_docs)]
        pub static #static_name: #struct_name = #struct_name;
    };

    expanded.into()
}
