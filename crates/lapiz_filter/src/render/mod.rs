use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result, bail, ensure};
use bevy_math::IRect;
use futures::channel::oneshot;
use iced_runtime::Task;
use indexmap::IndexMap;
use lapiz_effect::{
    asset::{EffectInputSlotId, EffectOutputSlotId},
    instance::{EffectInputSlot, EffectInputs},
    render::EffectRenderer,
};
use lapiz_image::{
    layer::LayerId,
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuTileStorage},
};
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::global::Globals;
use lapiz_shader_graph::{
    graph::{slot::GraphValueType as _, variable::GraphShaderLiteral},
    wgsl_std::types::handle::{LayerReference, LayerType, PreparedLayer, PreparedLayerPixels},
};
use parking_lot::Mutex;
use wgpu::{Device, Queue};

use crate::instance::{FilterInstance, FilterParameter};

pub mod graph;

/// A target canvas layer snapshot and pixel bounding rectangle prepared for [`FilterRenderer`] execution.
pub struct FilterLayer {
    pub id: LayerId,
    pub storage: DynamicLayerStorage,
    pub bounds: IRect,
}

struct FilterRendererInner {
    renderer: EffectRenderer,
    target_input: EffectInputSlotId,
    layer_output: EffectOutputSlotId,
    device: Device,
    queue: Queue,
    chain: Mutex<Option<oneshot::Receiver<()>>>,
}

/// Compiles and executes a [`FilterInstance`] across one or more canvas layers on the GPU.
///
/// `FilterRenderer` validates that the filter's underlying [`EffectInstance`](lapiz_effect::instance::EffectInstance)
/// declares exactly one [`LayerType`] input slot and one [`LayerType`] output slot, then compiles its
/// passes into an [`EffectRenderer`]. During execution, it binds each target [`FilterLayer`] alongside
/// the prepared [`FilterParameter`] uniforms, runs the compute pipeline sequentially per layer, and
/// serializes asynchronous invocations through an internal oneshot queue so concurrent parameter updates
/// never interleave GPU resources.
#[derive(Clone)]
pub struct FilterRenderer {
    inner: Arc<FilterRendererInner>,
}

impl FilterRenderer {
    #[tracing::instrument(skip_all, name = "new_filter_renderer")]
    pub fn new(globals: &Globals, instance: &FilterInstance) -> Result<Self> {
        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();
        Self::from_context(instance, device, queue)
    }

    pub fn from_context(instance: &FilterInstance, device: Device, queue: Queue) -> Result<Self> {
        let effect = instance.effect();
        let mut layer_inputs = effect
            .inputs
            .values()
            .filter(|slot| slot.ty.is::<LayerType>());
        let target = layer_inputs
            .next()
            .context("Filter effect has no layer input")?;
        ensure!(
            layer_outputs(layer_inputs).is_none(),
            "Filter effect must have exactly one layer input"
        );
        let mut layer_outputs = effect
            .outputs
            .values()
            .filter(|slot| slot.ty.is::<LayerType>());
        let output = layer_outputs
            .next()
            .context("Filter effect has no layer output")?;
        ensure!(
            layer_outputs.next().is_none(),
            "Filter effect must have exactly one layer output"
        );

        let renderer = EffectRenderer::from_instance(
            effect,
            HashMap::new(),
            &[],
            device.clone(),
            queue.clone(),
        )?;
        let (tx, rx) = oneshot::channel();
        tx.send(()).ok();
        Ok(Self {
            inner: Arc::new(FilterRendererInner {
                renderer,
                target_input: target.id,
                layer_output: output.id,
                device,
                queue,
                chain: Mutex::new(Some(rx)),
            }),
        })
    }

    pub fn run(
        &self,
        layer_ids: Vec<LayerId>,
        parameters: IndexMap<EffectInputSlotId, FilterParameter>,
        tile_storage: &GpuTileStorage,
        device: &Device,
        queue: &Queue,
    ) -> Task<Result<HashMap<LayerId, DynamicLayerStorage>>> {
        let mut layers = Vec::with_capacity(layer_ids.len());
        for layer_id in layer_ids {
            let storage = match tile_storage.get_layer(layer_id) {
                Some(layer) => layer.deep_clone(),
                None => {
                    return Task::done(Err(anyhow::anyhow!(
                        "Filter render failed: target layer storage unavailable for {layer_id}"
                    )));
                }
            };
            let mut tile_rect = IRect::EMPTY;
            for tile in tile_storage.get_layer_tiles(layer_id).unwrap_or_default() {
                tile_rect = tile_rect.union(IRect {
                    min: tile,
                    max: tile + bevy_math::IVec2::ONE,
                });
            }

            if tile_rect.is_empty() {
                continue;
            }

            layers.push(FilterLayer {
                id: layer_id,
                storage,
                bounds: GpuTileStorage::tile_rect_to_pixel(tile_rect),
            });
        }

        let parameters =
            parameters
                .into_iter()
                .try_fold(EffectInputs::new(), |mut acc, (id, parameter)| {
                    acc.insert(id, parameter.value.prepare_to_shader(device, queue)?);
                    Result::<_>::Ok(acc)
                });

        match parameters {
            Ok(parameters) => self.run_prepared(layers, parameters),
            Err(e) => Task::done(Err(e)),
        }
    }

    /// Runs the filter on already prepared layers and inputs; serialized so
    /// concurrent callers never interleave GPU work of the same renderer.
    pub fn run_prepared(
        &self,
        layers: Vec<FilterLayer>,
        parameters: EffectInputs,
    ) -> Task<Result<HashMap<LayerId, DynamicLayerStorage>>> {
        let inner = self.inner.clone();
        let (new_tx, new_rx) = oneshot::channel();
        let prev = {
            let mut guard = inner.chain.lock();
            (*guard).replace(new_rx).unwrap_or_else(|| {
                let (t, r) = oneshot::channel();
                t.send(()).ok();
                r
            })
        };
        Task::future(async move {
            let _ = prev.await;
            let result = inner.run_layers(&layers, parameters);
            new_tx.send(()).ok();
            result
        })
    }

    /// Direct entry for callers that manage their own scheduling (tests);
    /// skips the run serialization the panel relies on.
    pub fn run_layers(
        &self,
        layers: Vec<FilterLayer>,
        parameters: IndexMap<EffectInputSlotId, FilterParameter>,
    ) -> Result<HashMap<LayerId, DynamicLayerStorage>> {
        let inputs = parameters
            .into_iter()
            .map(|(id, parameter)| {
                Ok((
                    id,
                    parameter
                        .value
                        .prepare_to_shader(&self.inner.device, &self.inner.queue)?,
                ))
            })
            .collect::<Result<EffectInputs>>()?;
        self.inner.run_layers(&layers, inputs)
    }
}

impl FilterRendererInner {
    fn run_layers(
        &self,
        layers: &[FilterLayer],
        mut parameters: EffectInputs,
    ) -> Result<HashMap<LayerId, DynamicLayerStorage>> {
        let mut results = HashMap::with_capacity(layers.len());
        for layer in layers {
            let input = prepare_layer_input(
                layer.storage.deep_clone(),
                layer.bounds,
                &self.device,
                &self.queue,
            )?;
            parameters.insert(self.target_input, input);
            let mut outputs = self
                .renderer
                .run(&parameters, &HashMap::new())
                .context("Filter effect run failed")?;
            parameters.remove(&self.target_input);

            let literal = outputs
                .remove(&self.layer_output)
                .context("Filter effect did not produce its layer output")?;
            let prepared = literal.downcast::<PreparedLayer>();
            let PreparedLayerPixels::ReadWrite { storage, .. } = prepared.pixels else {
                bail!("filter produced a read-only layer");
            };
            results.insert(layer.id, storage);
        }
        Ok(results)
    }
}

fn layer_outputs<'a>(
    mut inputs: impl Iterator<Item = &'a EffectInputSlot>,
) -> Option<&'a EffectInputSlot> {
    inputs.next()
}

pub fn prepare_layer_input(
    storage: DynamicLayerStorage,
    bounds: IRect,
    device: &Device,
    queue: &Queue,
) -> Result<GraphShaderLiteral> {
    let ty = LayerType {
        texel_type: TexelType::RGBA8,
    };
    let mut prepared = ty.prepare_to_shader(&LayerReference, device, queue)?;
    let PreparedLayerPixels::ReadWrite {
        storage: prepared_storage,
        ..
    } = &mut prepared.pixels
    else {
        unreachable!("newly prepared layer should be writable");
    };
    *prepared_storage = storage;
    prepared.pixel_bounds = Some(bounds);
    queue.write_buffer(
        &prepared.bounds,
        0,
        bytemuck::cast_slice(&[bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]),
    );
    Ok(GraphShaderLiteral::new_non_default(prepared, ty))
}
