mod bind_group_layouts;
mod buffers;
pub mod culling;
pub mod mutable_bind_group;

pub mod assets;
pub mod instance;
pub mod permutations;
pub mod pipelines;
pub mod utils;

pub use arc_swap;
pub use bytemuck;
pub use glam;
pub use gltf_helpers;
pub use mutable_bind_group::MutableBindGroup;
pub use shared_structs;

pub use bind_group_layouts::BindGroupLayouts;
pub use buffers::{
    AllocatedBuffer, AnimatedVertexBuffers, IndexBuffer, RawAnimatedVertexBuffers,
    RawVertexBuffers, VecGpuBuffer, VertexBuffers,
};
pub use instance::{GpuInstance, Instance, LineVertex};
pub use pipelines::{PipelineOptions, Pipelines};

#[cfg(feature = "wasm")]
use wasm_bindgen::closure::Closure;

#[cfg(feature = "wasm")]
pub fn request_animation_frame(
    session: &web_sys::XrSession,
    f: &Closure<dyn FnMut(f64, web_sys::XrFrame)>,
) {
    use wasm_bindgen::JsCast;

    // This turns the Closure into a js_sys::Function
    // See https://rustwasm.github.io/wasm-bindgen/api/wasm_bindgen/closure/struct.Closure.html#casting-a-closure-to-a-js_sysfunction
    session.request_animation_frame(f.as_ref().unchecked_ref());
}

#[cfg(feature = "wasm")]
pub fn run_rendering_loop<F: FnMut(f64, web_sys::XrFrame) + 'static>(
    session: &web_sys::XrSession,
    mut func: F,
) {
    use std::cell::RefCell;
    use std::rc::Rc;

    // Wierd hacky closure stuff that I don't understand. Taken from a wasm-bindgen example:
    // https://github.com/rustwasm/wasm-bindgen/blob/ebe658739c075fe78781d87ee9aa46533922476d/examples/webxr/src/lib.rs#L119-L151
    let closure = Rc::new(RefCell::new(None));
    let closure_clone = closure.clone();

    *closure.borrow_mut() = Some(Closure::wrap(Box::new(
        move |time: f64, frame: web_sys::XrFrame| {
            let session = frame.session();

            request_animation_frame(&session, closure_clone.borrow().as_ref().unwrap());

            func(time, frame);
        },
    )
        as Box<dyn FnMut(f64, web_sys::XrFrame)>));

    request_animation_frame(session, closure.borrow().as_ref().unwrap());
}

pub struct ContextCreationOptions {
    pub stencil: bool,
}

#[cfg(feature = "wasm")]
pub struct Canvas {
    inner: web_sys::HtmlCanvasElement,
    id: u32,
}

#[cfg(feature = "wasm")]
impl Canvas {
    pub fn new_with_id(id: u32) -> Self {
        use wasm_bindgen::JsCast;

        let canvas: web_sys::HtmlCanvasElement = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("canvas")
            .unwrap()
            .unchecked_into();

        let body = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .body()
            .unwrap();

        canvas
            .set_attribute("data-raw-handle", &id.to_string())
            .unwrap();

        body.append_child(&web_sys::Element::from(canvas.clone()))
            .unwrap();

        Self { inner: canvas, id }
    }

    pub fn create_webgl2_context(
        &self,
        options: ContextCreationOptions,
    ) -> web_sys::WebGl2RenderingContext {
        use wasm_bindgen::JsCast;

        let js_gl_attribs = js_sys::Object::new();
        js_sys::Reflect::set(
            &js_gl_attribs,
            &"xrCompatible".into(),
            &wasm_bindgen::JsValue::TRUE,
        )
        .expect("Failed to set xrCompatible");
        // WebGL silently ignores any stencil writing or testing if this is not set.
        // (Atleast on Chrome). What a fantastic design decision.
        js_sys::Reflect::set(
            &js_gl_attribs,
            &"stencil".into(),
            &wasm_bindgen::JsValue::from_bool(options.stencil),
        )
        .expect("Failed to set stencil");

        self.inner
            .get_context_with_context_options("webgl2", &js_gl_attribs)
            .unwrap()
            .unwrap()
            .dyn_into::<web_sys::WebGl2RenderingContext>()
            .unwrap()
    }
}

#[cfg(feature = "wasm")]
impl Default for Canvas {
    fn default() -> Self {
        Self::new_with_id(0)
    }
}

#[cfg(feature = "wasm")]
unsafe impl raw_window_handle::HasRawWindowHandle for Canvas {
    fn raw_window_handle(&self) -> raw_window_handle::RawWindowHandle {
        let mut web = raw_window_handle::WebWindowHandle::empty();
        web.id = self.id;

        raw_window_handle::RawWindowHandle::Web(web)
    }
}

#[cfg(feature = "wasm")]
unsafe impl raw_window_handle::HasRawDisplayHandle for Canvas {
    fn raw_display_handle(&self) -> raw_window_handle::RawDisplayHandle {
        raw_window_handle::RawDisplayHandle::Web(raw_window_handle::WebDisplayHandle::empty())
    }
}

#[cfg(feature = "webgl")]
pub fn create_view_from_device_framebuffer(
    device: &wgpu::Device,
    framebuffer: web_sys::WebGlFramebuffer,
    base_layer: &web_sys::XrWebGlLayer,
    format: wgpu::TextureFormat,
    label: &'static str,
) -> Texture {
    Texture::new(unsafe {
        device.create_texture_from_hal::<wgpu_hal::gles::Api>(
            wgpu_hal::gles::Texture {
                inner: wgpu_hal::gles::TextureInner::ExternalFramebuffer { inner: framebuffer },
                mip_level_count: 1,
                array_layer_count: 1,
                format,
                format_desc: wgpu_hal::gles::TextureFormatDesc {
                    internal: glow::RGBA,
                    external: glow::RGBA,
                    data_type: glow::UNSIGNED_BYTE,
                },
                copy_size: wgpu_hal::CopyExtent {
                    width: base_layer.framebuffer_width(),
                    height: base_layer.framebuffer_height(),
                    depth: 1,
                },
                is_cubemap: false,
                drop_guard: None,
            },
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: base_layer.framebuffer_width(),
                    height: base_layer.framebuffer_height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        )
    })
}

#[derive(Debug)]
pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Texture {
    pub fn new(texture: wgpu::Texture) -> Self {
        Self {
            view: texture.create_view(&Default::default()),
            texture,
        }
    }

    pub fn new_with_view_dimension(
        texture: wgpu::Texture,
        dimension: wgpu::TextureViewDimension,
    ) -> Self {
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(dimension),
                ..Default::default()
            }),
            texture,
        }
    }

    pub fn new_cubemap(texture: wgpu::Texture) -> Self {
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            }),
            texture,
        }
    }
}

#[cfg(feature = "wasm")]
pub fn spawn<F: std::future::Future<Output = anyhow::Result<()>> + 'static>(future: F) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = future.await {
            log::error!("{}", error);
        }
    });
}

#[cfg(not(feature = "wasm"))]
pub fn spawn<F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static>(future: F)
where
    <F as std::future::Future>::Output: Send,
{
    async_std::task::spawn(async move {
        if let Err(error) = future.await {
            log::error!("{}", error);
        }
    });
}
