use wesl::syntax::{
    BinaryExpression, BinaryOperator, Expression, FunctionCall, Ident, LiteralExpression,
    NamedComponentExpression, Span, Spanned, TypeExpression,
};
use wesl_quote::quote_expression;

use crate::{
    graph::variable::GraphVariableCaster,
    wgsl_std::types::{
        primitive::{BoolType, F32Type, I32Type},
        vector::Vec2FType,
    },
};

/// Implicitly casts an [`F32Type`] scalar into a 2D [`Vec2FType`] vector by splatting `(x, x)`.
#[derive(Default, Clone)]
pub struct F32ToVec2FCaster;

impl GraphVariableCaster for F32ToVec2FCaster {
    type FromType = F32Type;

    type ToType = Vec2FType;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { vec2f(#variable, #variable) }
    }
}

/// Implicitly casts a [`Vec2FType`] vector into an [`F32Type`] scalar by extracting its `.x` component.
#[derive(Default, Clone)]
pub struct Vec2FToF32Caster;

impl GraphVariableCaster for Vec2FToF32Caster {
    type FromType = Vec2FType;

    type ToType = F32Type;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { #variable.x }
    }
}

/// Implicitly casts an [`F32Type`] floating-point scalar into an [`I32Type`] signed integer.
#[derive(Default, Clone)]
pub struct F32ToI32Caster;

impl GraphVariableCaster for F32ToI32Caster {
    type FromType = F32Type;

    type ToType = I32Type;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { i32(#variable) }
    }
}

/// Implicitly casts an [`I32Type`] signed integer into an [`F32Type`] floating-point scalar.
#[derive(Default, Clone)]
pub struct I32ToF32Caster;

impl GraphVariableCaster for I32ToF32Caster {
    type FromType = I32Type;

    type ToType = F32Type;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { f32(#variable) }
    }
}

/// Implicitly casts a [`BoolType`] boolean into an [`I32Type`] signed integer (`0i` or `1i`).
#[derive(Default, Clone)]
pub struct BoolToI32Caster;

impl GraphVariableCaster for BoolToI32Caster {
    type FromType = BoolType;

    type ToType = I32Type;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { select(0i, 1i, #variable) }
    }
}

/// Implicitly casts an [`I32Type`] signed integer into a [`BoolType`] boolean (`x == 1i`).
#[derive(Default, Clone)]
pub struct I32ToBoolCaster;

impl GraphVariableCaster for I32ToBoolCaster {
    type FromType = I32Type;

    type ToType = BoolType;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { #variable == 1i }
    }
}

/// Implicitly casts an [`I32Type`] signed integer into a 2D [`Vec2FType`] vector by splatting `f32(x)`.
#[derive(Default, Clone)]
pub struct I32ToVec2FCaster;

impl GraphVariableCaster for I32ToVec2FCaster {
    type FromType = I32Type;

    type ToType = Vec2FType;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { vec2f(f32(#variable)) }
    }
}

/// Implicitly casts a [`Vec2FType`] vector into an [`I32Type`] signed integer by converting its `.x` component.
#[derive(Default, Clone)]
pub struct Vec2FToI32Caster;

impl GraphVariableCaster for Vec2FToI32Caster {
    type FromType = Vec2FType;

    type ToType = I32Type;

    fn wgsl_cast(&self, variable: Expression) -> Expression {
        quote_expression! { i32(#variable.x) }
    }
}
