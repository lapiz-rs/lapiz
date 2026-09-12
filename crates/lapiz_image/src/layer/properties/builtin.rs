use anyhow::Result;
use lapiz_utils::wrapper;
use serde::{Deserialize, Serialize};

use crate::{
    blend_modes::BlendMode,
    composite::{BlendFunction, BlendFunctionId},
    layer::properties::{LayerProperties, LayerProperty},
    texel::TexelType,
};

wrapper! {
    #[derive(Debug, Clone, Copy)]
    pub VisibleProp : bool
}
impl LayerProperty for VisibleProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "visible"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(&self.0)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(rmp_serde::from_slice(data)?))
    }
}
impl Default for VisibleProp {
    fn default() -> Self {
        Self(true)
    }
}
pub trait VisiblePropertyExt {
    fn visible(&self) -> bool {
        self.get_visible().unwrap()
    }
    fn get_visible(&self) -> Option<bool>;
    fn set_visible(&mut self, visible: bool);
}
impl VisiblePropertyExt for LayerProperties {
    fn get_visible(&self) -> Option<bool> {
        Some(self.get::<VisibleProp>()?.0)
    }

    fn set_visible(&mut self, visible: bool) {
        self.set::<VisibleProp>(VisibleProp(visible));
    }
}

wrapper! {
    #[derive(Debug, Clone)]
    pub BlendFunctionProp : BlendFunctionId
}
impl LayerProperty for BlendFunctionProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "blend_function"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(&self.0)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(rmp_serde::from_slice(data)?))
    }
}
impl Default for BlendFunctionProp {
    fn default() -> Self {
        Self(BlendMode::Normal.id())
    }
}
pub trait BlendFunctionPropertyExt {
    fn blend_function(&self) -> &BlendFunctionId {
        self.get_blend_function().unwrap()
    }
    fn get_blend_function(&self) -> Option<&BlendFunctionId>;
    fn set_blend_function(&mut self, blend_function: BlendFunctionId);
}
impl BlendFunctionPropertyExt for LayerProperties {
    fn get_blend_function(&self) -> Option<&BlendFunctionId> {
        Some(&self.get::<BlendFunctionProp>()?.0)
    }

    fn set_blend_function(&mut self, blend_function: BlendFunctionId) {
        self.set::<BlendFunctionProp>(BlendFunctionProp(blend_function));
    }
}

wrapper! {
    #[derive(Debug, Clone, Copy)]
    pub OpacityProp : f32
}
impl LayerProperty for OpacityProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "opacity"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(&self.0)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(rmp_serde::from_slice(data)?))
    }
}
impl Default for OpacityProp {
    fn default() -> Self {
        Self(1.0)
    }
}
pub trait OpacityPropertyExt {
    fn opacity(&self) -> f32 {
        self.get_opacity().unwrap()
    }
    fn get_opacity(&self) -> Option<f32>;
    fn set_opacity(&mut self, opacity: f32);
}
impl OpacityPropertyExt for LayerProperties {
    fn get_opacity(&self) -> Option<f32> {
        Some(self.get::<OpacityProp>()?.0)
    }

    fn set_opacity(&mut self, opacity: f32) {
        self.set::<OpacityProp>(OpacityProp(opacity));
    }
}

wrapper! {
    #[derive(Debug, Clone, Default)]
    pub NameProp : String
}
impl LayerProperty for NameProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "name"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(&self.0)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(rmp_serde::from_slice(data)?))
    }
}
pub trait NamePropertyExt {
    fn name(&self) -> &str {
        self.get_name().unwrap()
    }
    fn get_name(&self) -> Option<&str>;
    fn set_name(&mut self, name: String);
}
impl NamePropertyExt for LayerProperties {
    fn get_name(&self) -> Option<&str> {
        Some(&self.get::<NameProp>()?.0)
    }

    fn set_name(&mut self, name: String) {
        self.set::<NameProp>(NameProp(name));
    }
}

wrapper! {
    #[derive(Debug, Clone, Copy, Default)]
    pub LockedProp : bool
}
impl LayerProperty for LockedProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "locked"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(&self.0)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(rmp_serde::from_slice(data)?))
    }
}
pub trait LockedPropertyExt {
    fn locked(&self) -> bool {
        self.get_locked().unwrap()
    }
    fn get_locked(&self) -> Option<bool>;
    fn set_locked(&mut self, locked: bool);
}
impl LockedPropertyExt for LayerProperties {
    fn get_locked(&self) -> Option<bool> {
        Some(self.get::<LockedProp>()?.0)
    }
    fn set_locked(&mut self, locked: bool) {
        self.set::<LockedProp>(LockedProp(locked));
    }
}

wrapper! {
    #[derive(Debug, Clone, Copy, Default)]
    pub DisabledChannelsProp : u32
}
impl LayerProperty for DisabledChannelsProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "disabled_channels"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(&self.0)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(rmp_serde::from_slice(data)?))
    }
}
pub trait DisabledChannelsPropertyExt {
    fn disabled_channels(&self) -> DisabledChannelsProp {
        self.get_disabled_channels().unwrap()
    }
    fn get_disabled_channels(&self) -> Option<DisabledChannelsProp>;
    fn set_disabled_channels(&mut self, channels: DisabledChannelsProp);
}
impl DisabledChannelsPropertyExt for LayerProperties {
    fn get_disabled_channels(&self) -> Option<DisabledChannelsProp> {
        Some(*self.get::<DisabledChannelsProp>()?)
    }

    fn set_disabled_channels(&mut self, channels: DisabledChannelsProp) {
        self.set(channels);
    }
}
impl DisabledChannelsProp {
    pub fn is_channel_disabled(&self, channel: u32) -> bool {
        (self.0 & (1 << channel)) != 0
    }

    pub fn set_channel_disabled(&mut self, channel: u32, disabled: bool) {
        if disabled {
            self.0 |= 1 << channel;
        } else {
            self.0 &= !(1 << channel);
        }
    }

    pub fn toggle_channel_disabled(&mut self, channel: u32) {
        self.set_channel_disabled(channel, !self.is_channel_disabled(channel));
    }
}

wrapper! {
    #[derive(Debug, Clone, Copy, Default)]
    pub LockedChannelsProp : u32
}
impl LayerProperty for LockedChannelsProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "locked_channels"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(&self.0)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self(rmp_serde::from_slice(data)?))
    }
}
pub trait LockedChannelsPropertyExt {
    fn locked_channels(&self) -> LockedChannelsProp {
        self.get_locked_channels().unwrap()
    }
    fn get_locked_channels(&self) -> Option<LockedChannelsProp>;
    fn set_locked_channels(&mut self, channels: LockedChannelsProp);
}
impl LockedChannelsPropertyExt for LayerProperties {
    fn get_locked_channels(&self) -> Option<LockedChannelsProp> {
        Some(*self.get::<LockedChannelsProp>()?)
    }

    fn set_locked_channels(&mut self, channels: LockedChannelsProp) {
        self.set(channels);
    }
}
impl LockedChannelsProp {
    pub fn is_channel_locked(&self, channel: u32) -> bool {
        (self.0 & (1 << channel)) != 0
    }

    pub fn set_channel_locked(&mut self, channel: u32, locked: bool) {
        if locked {
            self.0 |= 1 << channel;
        } else {
            self.0 &= !(1 << channel);
        }
    }

    pub fn toggle_channel_locked(&mut self, channel: u32) {
        self.set_channel_locked(channel, !self.is_channel_locked(channel));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TexelSource {
    Generated,
    DirectlyDefined,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LayerTexelProp {
    pub ty: TexelType,
    pub source: TexelSource,
}

impl LayerTexelProp {
    pub fn new(ty: TexelType, source: TexelSource) -> Self {
        Self { ty, source }
    }
}

impl LayerProperty for LayerTexelProp {
    fn ident() -> &'static str
    where
        Self: Sized,
    {
        "texel_type"
    }
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(self)?)
    }
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(rmp_serde::from_slice(data)?)
    }
}
pub trait LayerTexelTypePropertyExt {
    fn get_texel_prop(&self) -> Option<LayerTexelProp>;
    fn get_texel_type(&self) -> Option<TexelType>;
    fn get_texel_source(&self) -> Option<TexelSource>;
}
impl LayerTexelTypePropertyExt for LayerProperties {
    fn get_texel_prop(&self) -> Option<LayerTexelProp> {
        self.get::<LayerTexelProp>().cloned()
    }
    fn get_texel_type(&self) -> Option<TexelType> {
        self.get_texel_prop().map(|prop| prop.ty)
    }
    fn get_texel_source(&self) -> Option<TexelSource> {
        self.get_texel_prop().map(|prop| prop.source)
    }
}
