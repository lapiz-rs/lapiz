use indexmap::IndexMap;
use lapiz_assets::{
    asset::{AssetHandle, AssetId},
    store::AssetRegistry,
};
use lapiz_effect::{asset::EffectInputSlotId, instance::EffectInstance};
use lapiz_shader_graph::{
    graph::{GraphResources, slot::ErasedGraphLiteralUpdateMessage, variable::GraphLiteral},
    save::SerializableGraphLiteral,
    wgsl_std::types::handle::LayerType,
};

use crate::{
    asset::{FilterPreset, FilterPresetMetadata, SerializableFilterParameter},
    render::graph::filter_graph_resources,
};

/// A runtime user-adjustable filter parameter pairing a display name with a live [`GraphLiteral`] value.
#[derive(Clone)]
pub struct FilterParameter {
    pub name: String,
    pub value: GraphLiteral,
}

/// The live runtime state of a [`FilterPreset`] loaded for parameter adjustment, graph editing, or GPU execution.
///
/// `FilterInstance` wraps a live [`EffectInstance`] alongside resolved [`FilterParameter`] values for
/// every non-layer input slot of the effect. While the target [`LayerType`] input and output slots
/// are bound automatically by [`FilterRenderer`](crate::render::FilterRenderer) when processing canvas
/// layers, all remaining effect inputs are exposed as user-configurable parameters and kept synchronized
/// with the underlying [`EffectInstance`] signature.
pub struct FilterInstance {
    filter_id: Option<AssetId<FilterPreset>>,
    metadata: FilterPresetMetadata,
    effect: EffectInstance,
    parameters: IndexMap<EffectInputSlotId, FilterParameter>,
}

impl FilterInstance {
    pub fn from_asset(
        handle: &AssetHandle<FilterPreset>,
        assets: &AssetRegistry,
    ) -> anyhow::Result<Self> {
        let preset = handle
            .get()
            .map_err(|e| anyhow::anyhow!("Filter preset asset is not loaded yet: {e}"))?;
        let mut instance = Self::new(&preset, filter_graph_resources(assets.clone()))?;
        instance.filter_id = Some(handle.id());
        Ok(instance)
    }

    pub fn new(preset: &FilterPreset, resources: GraphResources) -> anyhow::Result<Self> {
        let effect = EffectInstance::from_asset(&preset.effect, resources.clone())?;

        let mut parameters = IndexMap::new();
        for (id, slot) in &effect.inputs {
            if let Some(persisted) = preset.parameters.get(id) {
                match persisted
                    .value
                    .deserialize(&resources.type_registry, &resources.assets)
                {
                    Ok(value) => {
                        parameters.insert(
                            *id,
                            FilterParameter {
                                name: slot.name.clone(),
                                value,
                            },
                        );
                        continue;
                    }
                    Err(e) => {
                        log::warn!(
                            "Filter parameter '{}' failed to deserialize, using default: {e}",
                            slot.name
                        );
                    }
                }
            }
            parameters.insert(
                *id,
                FilterParameter {
                    name: slot.name.clone(),
                    value: GraphLiteral::new_boxed_default(slot.ty.clone()),
                },
            );
        }

        for id in preset.parameters.keys() {
            if !effect.inputs.contains_key(id) {
                log::warn!("Filter preset has a parameter with no matching effect input");
            }
        }

        Ok(Self {
            filter_id: None,
            metadata: preset.metadata.clone(),
            effect,
            parameters,
        })
    }

    pub fn as_asset(&self, assets: &AssetRegistry) -> anyhow::Result<FilterPreset> {
        let mut parameters = IndexMap::new();
        for (id, parameter) in &self.parameters {
            parameters.insert(
                *id,
                SerializableFilterParameter {
                    name: parameter.name.clone(),
                    value: SerializableGraphLiteral::serialize(&parameter.value, assets)?,
                },
            );
        }
        Ok(FilterPreset {
            metadata: self.metadata.clone(),
            effect: self.effect.as_asset()?,
            parameters,
        })
    }

    pub fn asset_id(&self) -> Option<AssetId<FilterPreset>> {
        self.filter_id
    }

    pub fn metadata(&self) -> &FilterPresetMetadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut FilterPresetMetadata {
        &mut self.metadata
    }

    pub fn effect(&self) -> &EffectInstance {
        &self.effect
    }

    pub fn effect_mut(&mut self) -> &mut EffectInstance {
        &mut self.effect
    }

    pub fn parameters(&self) -> &IndexMap<EffectInputSlotId, FilterParameter> {
        &self.parameters
    }

    pub fn parameters_mut(&mut self) -> &mut IndexMap<EffectInputSlotId, FilterParameter> {
        &mut self.parameters
    }

    pub fn update_parameter(
        &mut self,
        id: &EffectInputSlotId,
        message: ErasedGraphLiteralUpdateMessage,
    ) {
        if let Some(parameter) = self.parameters.get_mut(id) {
            parameter.value.update(message);
        }
    }

    pub fn resync_parameters(&mut self) {
        self.parameters.retain(|id, _| {
            self.effect
                .inputs
                .get(id)
                .is_some_and(|slot| !slot.ty.is::<LayerType>())
        });
        for (id, slot) in &self.effect.inputs {
            if slot.ty.is::<LayerType>() {
                continue;
            }
            let parameter = self
                .parameters
                .entry(*id)
                .or_insert_with(|| FilterParameter {
                    name: slot.name.clone(),
                    value: GraphLiteral::new_boxed_default(slot.ty.clone()),
                });
            parameter.name = slot.name.clone();
        }
    }
}
