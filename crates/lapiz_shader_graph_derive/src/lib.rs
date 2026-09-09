use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{ItemImpl, parse_macro_input};

#[proc_macro_attribute]
pub fn stateless(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let crate_path = if std::env::var("CARGO_PKG_NAME").as_deref() == Ok("lapiz_shader_graph") {
        quote! { crate }
    } else {
        quote! { ::lapiz_shader_graph }
    };
    let impl_block = parse_macro_input!(item as ItemImpl);
    let graph_node_impl = generate_graph_node_impl(&impl_block, &crate_path);
    quote! {
        #impl_block
        #graph_node_impl
    }
    .into()
}

fn generate_graph_node_impl(impl_block: &ItemImpl, crate_path: &TokenStream2) -> TokenStream2 {
    let self_ty = &impl_block.self_ty;
    let generics = &impl_block.generics;
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let data_ty = impl_block
        .trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .and_then(|segment| match &segment.arguments {
            syn::PathArguments::AngleBracketed(arguments) => arguments.args.first().cloned(),
            _ => None,
        })
        .and_then(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(quote! { #ty }),
            _ => None,
        })
        .unwrap_or_else(|| quote! { Data });

    quote! {
        impl #impl_generics #crate_path::graph::node::GraphNode<#data_ty>
            for #self_ty
            #where_clause
        {
            type State = #crate_path::graph::node::StatelessState;
            type Message = #crate_path::graph::slot::ErasedGraphLiteralUpdateMessage;

            fn id(&self) -> &'static str {
                <Self as #crate_path::graph::node::StatelessCommonGraphNode<#data_ty>>::id(self)
            }

            fn default_state(
                &self,
                _ctx: #crate_path::graph::node::GraphNodeDefaultStateContext<'_, #data_ty>,
            ) -> Self::State {
                #crate_path::graph::node::StatelessState::default()
            }

            fn header_hue_chroma(&self) -> (f32, f32) {
                <Self as #crate_path::graph::node::StatelessCommonGraphNode<#data_ty>>::header_hue_chroma(self)
            }

            fn create_inputs(
                &self,
                _state: &Self::State,
                ctx: #crate_path::graph::node::GraphNodeCreateSlotsContext<'_, #data_ty>,
            ) -> ::std::vec::Vec<#crate_path::graph::slot::GraphDefaultInputSlot> {
                <Self as #crate_path::graph::node::StatelessCommonGraphNode<#data_ty>>::create_inputs(self, ctx)
            }

            fn create_outputs(
                &self,
                _state: &Self::State,
                ctx: #crate_path::graph::node::GraphNodeCreateSlotsContext<'_, #data_ty>,
            ) -> ::std::vec::Vec<#crate_path::graph::slot::GraphDefaultOutputSlot> {
                <Self as #crate_path::graph::node::StatelessCommonGraphNode<#data_ty>>::create_outputs(self, ctx)
            }

            fn view(
                &self,
                _state: &Self::State,
                ctx: #crate_path::graph::node::GraphNodeViewContext<'_, #data_ty>,
            ) -> #crate_path::GraphElement<'static, Self::Message> {
                ctx.view_all_slots(::std::convert::identity)
            }

            fn update(
                &self,
                _state: &mut Self::State,
                message: Self::Message,
                mut ctx: #crate_path::graph::node::GraphNodeUpdateContext<'_, #data_ty>,
            ) {
                ctx.update_literal(message);
            }

            fn generate_code(
                &self,
                _state: &Self::State,
                ctx: #crate_path::graph::node::GraphNodeCodeGenContext<'_, #data_ty>,
            ) -> ::std::result::Result<
                ::std::string::String,
                #crate_path::graph::node::GraphNodeCodeGenError,
            > {
                <Self as #crate_path::graph::node::StatelessCommonGraphNode<#data_ty>>::generate_code(self, ctx)
            }

            fn update_signature(
                &self,
                _state: &Self::State,
                ctx: #crate_path::graph::node::GraphNodeUpdateSignatureContext<'_, #data_ty>,
            ) {
                <Self as #crate_path::graph::node::StatelessCommonGraphNode<#data_ty>>::update_signature(self, ctx);
            }
        }
    }
}
