use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DataEnum, DataStruct, DeriveInput, Error, Expr, Fields, GenericArgument, Lit,
    LitInt, LitStr, PathArguments, Type, parse_macro_input,
};

#[proc_macro_derive(AbrClass, attributes(abr))]
pub fn derive_abr_class(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as DeriveInput), DeriveKind::Class)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(AbrObject, attributes(abr))]
pub fn derive_abr_object(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as DeriveInput), DeriveKind::Object)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(AbrEnum, attributes(abr))]
pub fn derive_abr_enum(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as DeriveInput), DeriveKind::Enum)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(AbrIntegerEnum, attributes(abr))]
pub fn derive_abr_integer_enum(input: TokenStream) -> TokenStream {
    expand(
        parse_macro_input!(input as DeriveInput),
        DeriveKind::IntegerEnum,
    )
    .unwrap_or_else(Error::into_compile_error)
    .into()
}

/// Kind of ABR descriptor derive macro being expanded.
#[derive(Clone, Copy)]
enum DeriveKind {
    Class,
    Object,
    Enum,
    IntegerEnum,
}

fn expand(input: DeriveInput, kind: DeriveKind) -> syn::Result<proc_macro2::TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            input.generics,
            "ABR derives do not support generic types",
        ));
    }

    let crate_path = match crate_name("lapiz_abr") {
        Ok(FoundCrate::Itself) => quote! { crate },
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            quote! { ::#ident }
        }
        Err(error) => {
            return Err(Error::new(
                Span::call_site(),
                format!("failed to locate lapiz_abr crate: {error}"),
            ));
        }
    };
    let attrs = AbrAttributes::parse(&input.attrs)?;

    match (kind, input.data) {
        (DeriveKind::Class, Data::Struct(data)) => {
            expand_class(input.ident, data, attrs, &crate_path)
        }
        (DeriveKind::Object, Data::Enum(data)) => {
            expand_object(input.ident, data, attrs, &crate_path)
        }
        (DeriveKind::Enum, Data::Enum(data)) => expand_enum(input.ident, data, attrs, &crate_path),
        (DeriveKind::IntegerEnum, Data::Enum(data)) => {
            expand_integer_enum(input.ident, data, attrs, &crate_path)
        }
        (DeriveKind::Class, _) => Err(Error::new_spanned(
            input.ident,
            "AbrClass can only be derived for a struct",
        )),
        (DeriveKind::Object, _) => Err(Error::new_spanned(
            input.ident,
            "AbrObject can only be derived for a class enum",
        )),
        (DeriveKind::Enum, _) => Err(Error::new_spanned(
            input.ident,
            "AbrEnum can only be derived for an enum",
        )),
        (DeriveKind::IntegerEnum, _) => Err(Error::new_spanned(
            input.ident,
            "AbrIntegerEnum can only be derived for an enum",
        )),
    }
}

fn expand_class(
    name: syn::Ident,
    data: DataStruct,
    attrs: AbrAttributes,
    crate_path: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    let class_id = attrs.class.ok_or_else(|| {
        Error::new_spanned(&name, "descriptor struct requires #[abr(class = \"...\")]")
    })?;
    if attrs.enum_type.is_some()
        || attrs.key.is_some()
        || attrs.string_value.is_some()
        || attrs.integer_value.is_some()
        || attrs.default.is_some()
    {
        return Err(Error::new_spanned(
            &name,
            "descriptor struct only supports #[abr(class = \"...\")]",
        ));
    }

    let Fields::Named(fields) = data.fields else {
        return Err(Error::new_spanned(
            name,
            "AbrClass only supports structs with named fields",
        ));
    };

    let mut keys = HashSet::new();
    let mut declarations = Vec::new();
    let mut match_arms = Vec::new();
    let mut initializers = Vec::new();

    for field in fields.named {
        let field_name = field.ident.expect("named field");
        let field_attrs = AbrAttributes::parse(&field.attrs)?;
        if field_attrs.class.is_some()
            || field_attrs.enum_type.is_some()
            || field_attrs.string_value.is_some()
            || field_attrs.integer_value.is_some()
        {
            return Err(Error::new_spanned(
                &field_name,
                "descriptor fields only support #[abr(key = \"...\")] and #[abr(default = ...)]",
            ));
        }
        let key = field_attrs.key.ok_or_else(|| {
            Error::new_spanned(
                &field_name,
                "descriptor field requires #[abr(key = \"...\")]",
            )
        })?;
        if !keys.insert(key.value()) {
            return Err(Error::new_spanned(key, "duplicate descriptor field key"));
        }

        let variable = format_ident!("field_{field_name}");
        let default = field_attrs.default;
        let option_inner = if let Type::Path(path) = &field.ty
            && let Some(segment) = path.path.segments.last()
            && segment.ident == "Option"
            && let PathArguments::AngleBracketed(arguments) = &segment.arguments
            && arguments.args.len() == 1
            && let Some(GenericArgument::Type(inner)) = arguments.args.first()
        {
            Some(inner)
        } else {
            None
        };
        if option_inner.is_some()
            && let Some(default) = &default
        {
            return Err(Error::new_spanned(
                default,
                "a descriptor field with #[abr(default = ...)] must use T instead of Option<T>",
            ));
        }
        let (value_type, optional) = option_inner.map_or((&field.ty, false), |inner| (inner, true));

        declarations.push(quote! {
            let mut #variable: ::core::option::Option<#value_type> = ::core::option::Option::None;
        });
        match_arms.push(quote! {
            #key => {
                if #variable.is_some() {
                    return ::core::result::Result::Err(#crate_path::__private::Error::msg(
                        ::std::format!(
                            "duplicate field {:?} in ABR descriptor class {:?} at desc offset {}",
                            #key,
                            <Self as #crate_path::descriptor::AbrClass>::CLASS_ID,
                            offset,
                        )
                    ));
                }
                #variable = ::core::option::Option::Some(
                    <#value_type as #crate_path::descriptor::AbrValue>::parse_value(
                        cursor,
                        value_type,
                        offset,
                    )?
                );
            }
        });

        if let Some(default) = default {
            initializers.push(quote! {
                #field_name: #variable.unwrap_or_else(|| #default)
            });
        } else if optional {
            initializers.push(quote! { #field_name: #variable });
        } else {
            initializers.push(quote! {
                #field_name: #variable.ok_or_else(|| #crate_path::__private::Error::msg(
                    ::std::format!(
                        "missing field {:?} in ABR descriptor class {:?}",
                        #key,
                        <Self as #crate_path::descriptor::AbrClass>::CLASS_ID,
                    )
                ))?
            });
        }
    }

    Ok(quote! {
        impl #crate_path::descriptor::AbrClass for #name {
            const CLASS_ID: &'static str = #class_id;
        }

        impl #crate_path::descriptor::AbrObject for #name {
            fn parse_with_header(
                cursor: &mut #crate_path::cursor::Cursor<'_>,
                class_id: ::std::string::String,
                entry_count: usize,
                header_offset: usize,
            ) -> #crate_path::__private::Result<Self> {
                if class_id != <Self as #crate_path::descriptor::AbrClass>::CLASS_ID {
                    return ::core::result::Result::Err(#crate_path::__private::Error::msg(
                        ::std::format!(
                            "expected ABR descriptor class {:?}, found {:?} at desc offset {}",
                            <Self as #crate_path::descriptor::AbrClass>::CLASS_ID,
                            class_id,
                            header_offset,
                        )
                    ));
                }
                #(#declarations)*

                for _ in 0..entry_count {
                    let key = cursor.read_descriptor_id()?;
                    let offset = cursor.position();
                    let value_type = cursor.read_ostype()?;
                    match key.as_str() {
                        #(#match_arms)*
                        _ => <Self as #crate_path::descriptor::AbrObject>::skip_value(
                            cursor,
                            value_type,
                            offset,
                        )?,
                    }
                }

                ::core::result::Result::Ok(Self {
                    #(#initializers),*
                })
            }
        }
    })
}

fn expand_object(
    name: syn::Ident,
    data: DataEnum,
    attrs: AbrAttributes,
    crate_path: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    if attrs.class.is_some()
        || attrs.enum_type.is_some()
        || attrs.key.is_some()
        || attrs.string_value.is_some()
        || attrs.integer_value.is_some()
        || attrs.default.is_some()
    {
        return Err(Error::new_spanned(
            &name,
            "AbrObject does not accept type-level #[abr(...)] attributes",
        ));
    }

    let mut class_ids = Vec::new();
    let mut dispatch = Vec::new();

    for variant in data.variants {
        let variant_attrs = AbrAttributes::parse(&variant.attrs)?;
        if variant_attrs.class.is_some() {
            return Err(Error::new_spanned(
                variant,
                "class enum variants obtain CLASS_ID from their inner type; remove #[abr(class = \"...\")]",
            ));
        }
        if variant_attrs.enum_type.is_some()
            || variant_attrs.key.is_some()
            || variant_attrs.string_value.is_some()
            || variant_attrs.integer_value.is_some()
            || variant_attrs.default.is_some()
        {
            return Err(Error::new_spanned(
                variant,
                "class enum variants do not accept #[abr(...)] attributes",
            ));
        }

        let Fields::Unnamed(fields) = variant.fields else {
            return Err(Error::new_spanned(
                variant.ident,
                "class enum variants must contain exactly one unnamed field",
            ));
        };
        if fields.unnamed.len() != 1 {
            return Err(Error::new_spanned(
                fields,
                "class enum variants must contain exactly one unnamed field",
            ));
        }

        let variant_name = variant.ident;
        let inner = &fields.unnamed[0].ty;
        class_ids.push(quote! { <#inner as #crate_path::descriptor::AbrClass>::CLASS_ID });
        dispatch.push(quote! {
            if class_id == <#inner as #crate_path::descriptor::AbrClass>::CLASS_ID {
                return ::core::result::Result::Ok(Self::#variant_name(
                    <#inner as #crate_path::descriptor::AbrObject>::parse_with_header(
                        cursor,
                        class_id,
                        entry_count,
                        header_offset,
                    )?
                ));
            }
        });
    }

    Ok(quote! {
        impl #crate_path::descriptor::AbrObject for #name {
            fn parse_with_header(
                cursor: &mut #crate_path::cursor::Cursor<'_>,
                class_id: ::std::string::String,
                entry_count: usize,
                header_offset: usize,
            ) -> #crate_path::__private::Result<Self> {
                let class_ids = [#(#class_ids),*];
                for (index, class_id_to_check) in class_ids.iter().enumerate()
                {
                    if class_ids[index + 1..].contains(class_id_to_check)
                    {
                        return ::core::result::Result::Err(#crate_path::__private::Error::msg(
                            ::std::format!(
                                "duplicate ABR descriptor class ID {:?} in class enum",
                                class_id_to_check,
                            )
                        ));
                    }
                }
                #(#dispatch)*
                ::core::result::Result::Err(#crate_path::__private::Error::msg(
                    ::std::format!(
                        "unsupported ABR descriptor class {:?} for {} at desc offset {}",
                        class_id,
                        ::core::stringify!(#name),
                        header_offset,
                    )
                ))
            }
        }
    })
}

fn expand_enum(
    name: syn::Ident,
    data: DataEnum,
    attrs: AbrAttributes,
    crate_path: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    if attrs.class.is_some()
        || attrs.key.is_some()
        || attrs.string_value.is_some()
        || attrs.integer_value.is_some()
        || attrs.default.is_some()
    {
        return Err(Error::new_spanned(
            &name,
            "descriptor enum only supports #[abr(enum_type = \"...\")]",
        ));
    }
    let enum_type = attrs
        .enum_type
        .ok_or_else(|| Error::new_spanned(&name, "AbrEnum requires #[abr(enum_type = \"...\")]"))?;
    let mut values = HashSet::new();
    let mut arms = Vec::new();

    for variant in data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new_spanned(
                variant,
                "descriptor enum variants must not contain fields",
            ));
        }
        let variant_attrs = AbrAttributes::parse(&variant.attrs)?;
        if variant_attrs.class.is_some()
            || variant_attrs.enum_type.is_some()
            || variant_attrs.key.is_some()
            || variant_attrs.integer_value.is_some()
            || variant_attrs.default.is_some()
        {
            return Err(Error::new_spanned(
                &variant.ident,
                "descriptor enum variants only support #[abr(value = \"...\")]",
            ));
        }
        let value = variant_attrs.string_value.ok_or_else(|| {
            Error::new_spanned(
                &variant.ident,
                "descriptor enum variant requires #[abr(value = \"...\")]",
            )
        })?;
        if !values.insert(value.value()) {
            return Err(Error::new_spanned(value, "duplicate descriptor enum value"));
        }
        let variant_name = variant.ident;
        arms.push(quote! { #value => ::core::option::Option::Some(Self::#variant_name) });
    }

    Ok(quote! {
        impl #crate_path::descriptor::AbrValue for #name {
            fn parse_value(
                cursor: &mut #crate_path::cursor::Cursor<'_>,
                value_type: [u8; 4],
                offset: usize,
            ) -> #crate_path::__private::Result<Self> {
                <Self as #crate_path::descriptor::AbrEnum>::parse_enum_value(cursor, value_type, offset)
            }
        }

        impl #crate_path::descriptor::AbrEnum for #name {
            const TYPE_ID: &'static str = #enum_type;

            fn from_value_id(value_id: &str) -> ::core::option::Option<Self> {
                match value_id {
                    #(#arms,)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    })
}

fn expand_integer_enum(
    name: syn::Ident,
    data: DataEnum,
    attrs: AbrAttributes,
    crate_path: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    if attrs.class.is_some()
        || attrs.enum_type.is_some()
        || attrs.key.is_some()
        || attrs.string_value.is_some()
        || attrs.integer_value.is_some()
        || attrs.default.is_some()
    {
        return Err(Error::new_spanned(
            &name,
            "AbrIntegerEnum does not accept type-level #[abr(...)] attributes",
        ));
    }
    let mut values = HashSet::new();
    let mut arms = Vec::new();

    for variant in data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new_spanned(
                variant,
                "integer enum variants must not contain fields",
            ));
        }
        let variant_attrs = AbrAttributes::parse(&variant.attrs)?;
        if variant_attrs.class.is_some()
            || variant_attrs.enum_type.is_some()
            || variant_attrs.key.is_some()
            || variant_attrs.string_value.is_some()
            || variant_attrs.default.is_some()
        {
            return Err(Error::new_spanned(
                &variant.ident,
                "integer enum variants only support #[abr(value = 0)]",
            ));
        }
        let value = variant_attrs.integer_value.ok_or_else(|| {
            Error::new_spanned(
                &variant.ident,
                "integer enum variant requires #[abr(value = 0)]",
            )
        })?;
        let parsed = value.base10_parse::<i32>()?;
        if !values.insert(parsed) {
            return Err(Error::new_spanned(value, "duplicate integer enum value"));
        }
        let variant_name = variant.ident;
        arms.push(quote! { #parsed => ::core::option::Option::Some(Self::#variant_name) });
    }

    Ok(quote! {
        impl #crate_path::descriptor::AbrValue for #name {
            fn parse_value(
                cursor: &mut #crate_path::cursor::Cursor<'_>,
                value_type: [u8; 4],
                offset: usize,
            ) -> #crate_path::__private::Result<Self> {
                <Self as #crate_path::descriptor::AbrIntegerEnum>::parse_integer_value(cursor, value_type, offset)
            }
        }

        impl #crate_path::descriptor::AbrIntegerEnum for #name {
            fn from_i32(value: i32) -> ::core::option::Option<Self> {
                match value {
                    #(#arms,)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    })
}

/// Parsed `#[abr(...)]` helper attributes on a descriptor type, field, or variant.
#[derive(Default)]
struct AbrAttributes {
    class: Option<LitStr>,
    enum_type: Option<LitStr>,
    key: Option<LitStr>,
    string_value: Option<LitStr>,
    integer_value: Option<LitInt>,
    default: Option<Expr>,
}

impl AbrAttributes {
    fn parse(attributes: &[Attribute]) -> syn::Result<Self> {
        let mut result = Self::default();

        for attribute in attributes.iter().filter(|attr| attr.path().is_ident("abr")) {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("class") {
                    if result.class.replace(meta.value()?.parse()?).is_some() {
                        return Err(meta.error("duplicate abr class attribute"));
                    }
                } else if meta.path.is_ident("enum_type") {
                    if result.enum_type.replace(meta.value()?.parse()?).is_some() {
                        return Err(meta.error("duplicate abr enum_type attribute"));
                    }
                } else if meta.path.is_ident("key") {
                    if result.key.replace(meta.value()?.parse()?).is_some() {
                        return Err(meta.error("duplicate abr key attribute"));
                    }
                } else if meta.path.is_ident("value") {
                    if result.string_value.is_some() || result.integer_value.is_some() {
                        return Err(meta.error("duplicate abr value attribute"));
                    }
                    let literal = meta.value()?.parse::<Lit>()?;
                    match literal {
                        Lit::Str(value) => result.string_value = Some(value),
                        Lit::Int(value) => result.integer_value = Some(value),
                        _ => {
                            return Err(meta.error("abr value must be a string or integer literal"));
                        }
                    }
                } else if meta.path.is_ident("default") {
                    if result.default.replace(meta.value()?.parse()?).is_some() {
                        return Err(meta.error("duplicate abr default attribute"));
                    }
                } else {
                    return Err(meta.error("unsupported abr attribute"));
                }
                Ok(())
            })?;
        }

        Ok(result)
    }
}
