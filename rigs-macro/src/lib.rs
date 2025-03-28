use proc_macro::TokenStream;

mod tool;

#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    tool::tool_impl(attr, item)
}
