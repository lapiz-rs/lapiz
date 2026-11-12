use std::ops::RangeBounds;

use anyhow::Result;
use encase::{ShaderType, internal::CreateFrom};
use futures::{
    channel::oneshot::{self, Receiver, Sender},
    executor,
};
use iced_runtime::Task;
use lapiz_utils::{Deref, DerefMut};
use wgpu::{
    Buffer, BufferAddress, BufferAsyncError, BufferUsages, CommandEncoder, Device, Extent3d,
    MapMode, TexelCopyBufferInfo, TexelCopyBufferLayout, Texture,
};

pub fn create_readback_buffer_and_schedule_copy_texture(
    device: &Device,
    ec: &mut CommandEncoder,
    src_texture: &Texture,
) -> Buffer {
    let pixel_size = src_texture.format().block_copy_size(None).unwrap();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (src_texture.width() * src_texture.height() * pixel_size) as u64,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    ec.copy_texture_to_buffer(
        src_texture.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(src_texture.width() * pixel_size),
                rows_per_image: Some(src_texture.height()),
            },
        },
        Extent3d {
            width: src_texture.width(),
            height: src_texture.height(),
            depth_or_array_layers: src_texture.depth_or_array_layers(),
        },
    );
    buffer
}

pub fn create_readback_buffer_and_schedule_copy_buffer(
    device: &Device,
    ec: &mut CommandEncoder,
    src_buffer: &Buffer,
) -> Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: src_buffer.size(),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    ec.copy_buffer_to_buffer(src_buffer, 0, &buffer, 0, src_buffer.size());
    buffer
}

/// Handle to an in-flight asynchronous GPU buffer readback operation.
///
/// Wraps a oneshot [`Receiver`] that resolves to the decoded value `T` once the staging
/// [`Buffer`] has been mapped, copied, and unmapped. Results can be awaited asynchronously,
/// converted into an Iced [`Task`] via [`AsyncBufferReadback::into_task`], or blocked on
/// synchronously via [`AsyncBufferReadback::block_on`].
#[derive(Deref, DerefMut)]
pub struct AsyncBufferReadback<T> {
    rx: Receiver<anyhow::Result<T>>,
}

impl<T> AsyncBufferReadback<T> {
    pub fn block_on(self) -> anyhow::Result<T> {
        executor::block_on(self.rx)?
    }

    pub fn into_inner(self) -> Receiver<anyhow::Result<T>> {
        self.rx
    }
}

impl<T: Send + 'static> AsyncBufferReadback<T> {
    pub fn into_task(self) -> Task<Result<T>> {
        Task::future(async move {
            match self.rx.await {
                Ok(r) => r,
                Err(e) => Err(e.into()),
            }
        })
    }
}

pub fn readback_buffer_raw_on_submit_async<S>(
    ec: &mut CommandEncoder,
    buffer: &Buffer,
    bounds: S,
) -> AsyncBufferReadback<Vec<u8>>
where
    S: RangeBounds<BufferAddress> + Clone + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    ec.map_buffer_on_submit(
        buffer,
        MapMode::Read,
        bounds.clone(),
        on_buffer_mapped_raw(buffer.clone(), bounds, tx),
    );
    AsyncBufferReadback { rx }
}

pub fn readback_buffer_on_submit_async<T, S>(
    ec: &mut CommandEncoder,
    buffer: &Buffer,
    bounds: S,
) -> AsyncBufferReadback<T>
where
    T: ShaderType + CreateFrom + Send + 'static,
    S: RangeBounds<BufferAddress> + Clone + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    ec.map_buffer_on_submit(
        buffer,
        MapMode::Read,
        bounds.clone(),
        on_buffer_mapped(buffer.clone(), bounds, tx),
    );
    AsyncBufferReadback { rx }
}

pub fn readback_buffer_async<T, S>(buffer: &Buffer, bounds: S) -> AsyncBufferReadback<T>
where
    T: ShaderType + CreateFrom + Send + 'static,
    S: RangeBounds<BufferAddress> + Clone + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    buffer.map_async(
        MapMode::Read,
        bounds.clone(),
        on_buffer_mapped(buffer.clone(), bounds, tx),
    );
    AsyncBufferReadback { rx }
}

fn on_buffer_mapped<T, S>(
    buffer: Buffer,
    bounds: S,
    tx: Sender<anyhow::Result<T>>,
) -> impl FnOnce(Result<(), BufferAsyncError>)
where
    T: ShaderType + CreateFrom + Send + 'static,
    S: RangeBounds<BufferAddress> + Send + 'static,
{
    move |r| {
        if let Err(e) = r {
            buffer.unmap();
            tx.send(Err(e.into())).ok();
            return;
        }

        let bytes = buffer.get_mapped_range(bounds).to_vec();
        buffer.unmap();
        let mut wrapper = encase::DynamicStorageBuffer::new(bytes);
        let data = wrapper.create::<T>();
        tx.send(data.map_err(Into::into)).ok();
    }
}

fn on_buffer_mapped_raw<S>(
    buffer: Buffer,
    bounds: S,
    tx: Sender<anyhow::Result<Vec<u8>>>,
) -> impl FnOnce(Result<(), BufferAsyncError>)
where
    S: RangeBounds<BufferAddress> + Send + 'static,
{
    move |r| {
        if let Err(e) = r {
            buffer.unmap();
            tx.send(Err(e.into())).ok();
            return;
        }

        let bytes = buffer.get_mapped_range(bounds).to_vec();
        buffer.unmap();
        tx.send(Ok(bytes)).ok();
    }
}
