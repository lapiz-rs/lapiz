use std::env;

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemImpl, parse_macro_input};

#[proc_macro_attribute]
pub fn stateless(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let graph_path = if env::var("CARGO_CRATE_NAME").as_deref() == Ok("lapiz_effect") {
        quote! { crate::shader_graph }
    } else {
        quote! { ::lapiz_effect::shader_graph }
    };
    let impl_block = parse_macro_input!(item as ItemImpl);
    let graph_node_impl = generate_graph_node_impl(&impl_block, &graph_path);
    quote! {
        #impl_block
        #graph_node_impl
    }
    .into()
}

fn generate_graph_node_impl(
    impl_block: &ItemImpl,
    graph_path: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let self_ty = &impl_block.self_ty;
    let generics = &impl_block.generics;
    let (impl_generics, _, where_clause) = generics.split_for_impl();

    quote! {
        impl #impl_generics #graph_path::graph::node::GraphNode
            for #self_ty
            #where_clause
        {
            type State = #graph_path::graph::node::StatelessState;
            type Message = #graph_path::graph::slot::ErasedGraphLiteralUpdateMessage;

            fn id(&self) -> &'static str {
                <Self as #graph_path::graph::node::StatelessCommonGraphNode>::id(self)
            }

            fn default_state(
                &self,
                _ctx: #graph_path::graph::node::GraphNodeDefaultStateContext<'_>,
            ) -> Self::State {
                #graph_path::graph::node::StatelessState::default()
            }

            fn header_hue_chroma(&self) -> (f32, f32) {
                <Self as #graph_path::graph::node::StatelessCommonGraphNode>::header_hue_chroma(self)
            }

            fn create_inputs(
                &self,
                _state: &Self::State,
                ctx: #graph_path::graph::node::GraphNodeCreateSlotsContext<'_>,
            ) -> ::std::vec::Vec<#graph_path::graph::slot::GraphDefaultInputSlot> {
                <Self as #graph_path::graph::node::StatelessCommonGraphNode>::create_inputs(self, ctx)
            }

            fn create_outputs(
                &self,
                _state: &Self::State,
                ctx: #graph_path::graph::node::GraphNodeCreateSlotsContext<'_>,
            ) -> ::std::vec::Vec<#graph_path::graph::slot::GraphDefaultOutputSlot> {
                <Self as #graph_path::graph::node::StatelessCommonGraphNode>::create_outputs(self, ctx)
            }

            fn view<'a>(
                &self,
                _state: &'a Self::State,
                ctx: #graph_path::graph::node::GraphNodeViewContext<'a>,
            ) -> #graph_path::GraphElement<'a, Self::Message> {
                ctx.view_all_slots(::std::convert::identity)
            }

            fn update(
                &self,
                _state: &mut Self::State,
                message: Self::Message,
                mut ctx: #graph_path::graph::node::GraphNodeUpdateContext<'_>,
            ) {
                ctx.update_literal(message);
            }

            fn extra_shader_body(
                &self,
                _state: &Self::State,
                ctx: #graph_path::graph::node::GraphNodeInjectShaderBodyContext<'_>,
            ) {
                <Self as #graph_path::graph::node::StatelessCommonGraphNode>::extra_shader_body(self, ctx);
            }

            fn generate_code(
                &self,
                _state: &Self::State,
                ctx: #graph_path::graph::node::GraphNodeCodeGenContext<'_>,
            ) -> ::std::result::Result<
                ::std::string::String,
                #graph_path::graph::node::GraphNodeCodeGenError,
            > {
                <Self as #graph_path::graph::node::StatelessCommonGraphNode>::generate_code(self, ctx)
            }

            fn update_signature(
                &self,
                _state: &Self::State,
                ctx: #graph_path::graph::node::GraphNodeUpdateSignatureContext<'_>,
            ) {
                <Self as #graph_path::graph::node::StatelessCommonGraphNode>::update_signature(self, ctx);
            }
        }
    }
}
