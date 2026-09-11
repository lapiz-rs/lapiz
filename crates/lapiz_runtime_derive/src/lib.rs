use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(Event)]
pub fn derive_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let static_name = format_ident!("__{}_EVENT_CHANNEL", name.to_string().to_uppercase());

    quote! {
        static #static_name: ::lapiz_runtime::__private::LazyLock<(
            ::lapiz_runtime::__private::Sender<#name>,
            ::lapiz_runtime::__private::InactiveReceiver<#name>,
        )> = ::lapiz_runtime::__private::LazyLock::new(
            ::lapiz_runtime::__private::event_channel::<#name>,
        );

        impl ::lapiz_runtime::event::Event for #name {
            fn channel() -> &'static ::lapiz_runtime::__private::LazyLock<(
                ::lapiz_runtime::__private::Sender<#name>,
                ::lapiz_runtime::__private::InactiveReceiver<#name>,
            )> {
                &#static_name
            }
        }
    }
    .into()
}
