use wgpu::{
    BindGroupEntry, BindingResource, Buffer, BufferAddress, BufferBinding, BufferSize, Sampler,
    TextureView,
};

/// Owned counterpart to [`BindingResource`] that holds cloned `wgpu` handles without lifetime parameters.
#[derive(Clone, Debug)]
pub enum OwnedBindingResource {
    Buffer(OwnedBufferBinding),
    Sampler(Sampler),
    TextureView(TextureView),
}

impl OwnedBindingResource {
    pub fn as_borrowed<'a>(&'a self) -> BindingResource<'a> {
        match self {
            OwnedBindingResource::Buffer(buffer) => BindingResource::Buffer(buffer.as_borrowed()),
            OwnedBindingResource::Sampler(sampler) => BindingResource::Sampler(sampler),
            OwnedBindingResource::TextureView(texture_view) => {
                BindingResource::TextureView(texture_view)
            }
        }
    }

    pub fn from_borrowed(resource: BindingResource<'_>) -> Self {
        match resource {
            BindingResource::Buffer(buffer) => {
                OwnedBindingResource::Buffer(OwnedBufferBinding::from_borrowed(buffer))
            }
            BindingResource::Sampler(sampler) => OwnedBindingResource::Sampler(sampler.clone()),
            BindingResource::TextureView(texture_view) => {
                OwnedBindingResource::TextureView(texture_view.clone())
            }
            _ => panic!("unsupported"),
        }
    }
}

/// Owned counterpart to [`BufferBinding`] storing a cloned [`Buffer`], byte offset, and optional binding size.
#[derive(Clone, Debug)]
pub struct OwnedBufferBinding {
    pub buffer: Buffer,
    pub offset: BufferAddress,
    pub size: Option<BufferSize>,
}

impl OwnedBufferBinding {
    pub fn as_borrowed<'a>(&'a self) -> BufferBinding<'a> {
        BufferBinding {
            buffer: &self.buffer,
            offset: self.offset,
            size: self.size,
        }
    }

    pub fn from_borrowed(binding: BufferBinding<'_>) -> Self {
        Self {
            buffer: binding.buffer.clone(),
            offset: binding.offset,
            size: binding.size,
        }
    }
}

/// Owned counterpart to [`BindGroupEntry`] pairing a binding index with an [`OwnedBindingResource`].
#[derive(Clone, Debug)]
pub struct OwnedBindGroupEntry {
    pub binding: u32,
    pub resource: OwnedBindingResource,
}

impl OwnedBindGroupEntry {
    pub fn as_borrowed<'a>(&'a self) -> BindGroupEntry<'a> {
        BindGroupEntry {
            binding: self.binding,
            resource: self.resource.as_borrowed(),
        }
    }

    pub fn from_borrowed(entry: BindGroupEntry<'_>) -> Self {
        Self {
            binding: entry.binding,
            resource: OwnedBindingResource::from_borrowed(entry.resource),
        }
    }
}
