#![cfg(windows)]
use cursorcue_core::{Cursor, Point};
use windows::{
    Win32::{
        Foundation::{E_FAIL, HMODULE, HWND},
        Graphics::{
            Direct3D::{Fxc::D3DCompile, *},
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
    core::{Error, Interface, Result, s},
};

fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(|| Error::new(E_FAIL, "Direct3D did not return a resource"))
}
fn fit_cursor(position: Point, source: (u32, u32), scale: f32, style: u32) -> (Point, f32) {
    // Keep the full silhouette visible; only the visual hotspot is inset at frame edges.
    let (left, top, right, bottom) = match style {
        1 => (10.0, 10.0, 10.0, 10.0),
        2 => (14.0, 14.0, 14.0, 14.0),
        _ => (1.0, 1.0, 23.0, 36.0),
    };
    let width = source.0.max(1) as f32;
    let height = source.1.max(1) as f32;
    let scale = scale
        .max(0.01)
        .min(width / (left + right))
        .min(height / (top + bottom));
    (
        Point {
            x: position
                .x
                .clamp(left * scale, (width - right * scale).max(left * scale)),
            y: position
                .y
                .clamp(top * scale, (height - bottom * scale).max(top * scale)),
        },
        scale,
    )
}

pub struct Renderer {
    pub device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap: IDXGISwapChain1,
    target: Option<ID3D11RenderTargetView>,
    texture: Option<ID3D11Texture2D>,
    view: Option<ID3D11ShaderResourceView>,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    constants: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    source_size: (u32, u32),
    output_size: (u32, u32),
    appearance: [f32; 3],
    last_draw: Option<[f32; 12]>,
    source_dirty: bool,
}

impl Renderer {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        // SAFETY: OS-owned HWND is valid on this thread; out-pointers and descriptions live for each call.
        unsafe {
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device: ID3D11Device = required(device)?;
            let context = required(context)?;
            let dxgi: IDXGIDevice = device.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
            let shader = include_bytes!("composite.hlsl");
            let (mut vs, mut ps) = (None, None);
            let mut errors: Option<ID3DBlob> = None;
            D3DCompile(
                shader.as_ptr().cast(),
                shader.len(),
                s!("CursorCue"),
                None,
                None,
                s!("vsMain"),
                s!("vs_5_0"),
                0,
                0,
                &mut vs,
                Some(&mut errors),
            )
            .map_err(|error| shader_error(error, errors.as_ref()))?;
            D3DCompile(
                shader.as_ptr().cast(),
                shader.len(),
                s!("CursorCue"),
                None,
                None,
                s!("psMain"),
                s!("ps_5_0"),
                0,
                0,
                &mut ps,
                Some(&mut errors),
            )
            .map_err(|error| shader_error(error, errors.as_ref()))?;
            let (vs, ps) = (required(vs)?, required(ps)?);
            let mut vertex = None;
            let mut pixel = None;
            device.CreateVertexShader(
                std::slice::from_raw_parts(vs.GetBufferPointer().cast(), vs.GetBufferSize()),
                None,
                Some(&mut vertex),
            )?;
            device.CreatePixelShader(
                std::slice::from_raw_parts(ps.GetBufferPointer().cast(), ps.GetBufferSize()),
                None,
                Some(&mut pixel),
            )?;
            let mut constants = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: 48,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut constants),
            )?;
            let mut sampler = None;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxLOD: f32::MAX,
                    ComparisonFunc: D3D11_COMPARISON_NEVER,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            let mut result = Self {
                device,
                context,
                swap,
                target: None,
                texture: None,
                view: None,
                vertex: required(vertex)?,
                pixel: required(pixel)?,
                constants: required(constants)?,
                sampler: required(sampler)?,
                source_size: (0, 0),
                output_size: (width, height),
                appearance: [1.0, 1.0, 0.0],
                last_draw: None,
                source_dirty: true,
            };
            result.create_target()?;
            Ok(result)
        }
    }
    fn create_target(&mut self) -> Result<()> {
        // SAFETY: swap-chain COM resource and out-pointer are valid; the view owns its buffer reference.
        unsafe {
            let buffer: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            self.device
                .CreateRenderTargetView(&buffer, None, Some(&mut self.target))
        }
    }
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || self.output_size == (width, height) {
            return Ok(());
        }
        // SAFETY: unbind and release buffer references before resizing; all calls are on the render thread.
        unsafe {
            self.context.OMSetRenderTargets(None, None);
            self.target = None;
            self.swap.ResizeBuffers(
                2,
                width,
                height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.output_size = (width, height);
        self.invalidate();
        self.create_target()
    }
    pub fn update_source(&mut self, source: &ID3D11Texture2D) -> Result<()> {
        // SAFETY: capture and renderer use this same D3D11 device. GPU copies never map full frames to CPU.
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut desc);
            if self.source_size != (desc.Width, desc.Height) {
                self.view = None;
                self.texture = None;
                desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
                desc.Usage = D3D11_USAGE_DEFAULT;
                desc.CPUAccessFlags = 0;
                desc.MiscFlags = 0;
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut self.texture))?;
                let texture = required(self.texture.clone())?;
                self.device
                    .CreateShaderResourceView(&texture, None, Some(&mut self.view))?;
                self.source_size = (desc.Width, desc.Height);
            }
            self.context.PSSetShaderResources(0, Some(&[None]));
            self.context
                .CopyResource(&required(self.texture.clone())?, source);
        }
        self.source_dirty = true;
        Ok(())
    }
    pub fn render(&mut self, cursor: &Cursor) -> Result<bool> {
        if self.view.is_none() {
            return Ok(false);
        }
        let state = self.draw_state(cursor);
        if !self.source_dirty && self.last_draw == Some(state) {
            return Ok(false);
        }
        self.draw(cursor)?;
        // SAFETY: the swap chain belongs to the current rendering thread.
        unsafe {
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        self.last_draw = Some(state);
        self.source_dirty = false;
        Ok(true)
    }
    fn draw_state(&self, cursor: &Cursor) -> [f32; 12] {
        let (position, scale) = fit_cursor(
            cursor.position,
            self.source_size,
            self.appearance[0],
            self.appearance[2] as u32,
        );
        let mut values: [f32; 12] = [
            self.source_size.0 as f32,
            self.source_size.1 as f32,
            self.output_size.0 as f32,
            self.output_size.1 as f32,
            position.x,
            position.y,
            if cursor.visible() { 1.0 } else { 0.0 },
            scale,
            self.appearance[1],
            self.appearance[2],
            0.0,
            0.0,
        ];
        if !cursor.visible() {
            values[4..10].fill(0.0);
        }
        values
    }
    fn draw(&self, cursor: &Cursor) -> Result<()> {
        if self.view.is_none() {
            return Ok(());
        }
        let values = self.draw_state(cursor);
        // SAFETY: buffer update copies exactly 48 bytes; valid COM resources are bound on their owning thread.
        unsafe {
            self.context
                .UpdateSubresource(&self.constants, 0, None, values.as_ptr().cast(), 0, 0);
            self.context
                .OMSetRenderTargets(Some(std::slice::from_ref(&self.target)), None);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: self.output_size.0 as f32,
                Height: self.output_size.1 as f32,
                MaxDepth: 1.0,
                ..Default::default()
            }]));
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex, None);
            self.context.PSSetShader(&self.pixel, None);
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context
                .PSSetShaderResources(0, Some(std::slice::from_ref(&self.view)));
            self.context.Draw(3, 0);
            Ok(())
        }
    }
    pub fn diagnostic_pixel(
        &mut self,
        cursor: &Cursor,
        point: cursorcue_core::Point,
    ) -> Result<[u8; 4]> {
        self.invalidate();
        if self.view.is_none()
            || !point.x.is_finite()
            || !point.y.is_finite()
            || point.x < 0.0
            || point.y < 0.0
        {
            return Err(Error::new(
                E_FAIL,
                "Diagnostic pixel requires a valid source and coordinates",
            ));
        }
        self.draw(cursor)?;
        let scale = (self.output_size.0 as f32 / self.source_size.0 as f32)
            .min(self.output_size.1 as f32 / self.source_size.1 as f32);
        let x = ((self.output_size.0 as f32 - self.source_size.0 as f32 * scale) * 0.5
            + point.x * scale)
            .floor() as u32;
        let y = ((self.output_size.1 as f32 - self.source_size.1 as f32 * scale) * 0.5
            + point.y * scale)
            .floor() as u32;
        if x >= self.output_size.0 || y >= self.output_size.1 {
            return Err(Error::new(
                E_FAIL,
                "Diagnostic pixel is outside the output frame",
            ));
        }
        // SAFETY: diagnostic reads one pixel only, never screen-size bitmaps. The mapped four bytes are copied before Unmap.
        unsafe {
            let buffer: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            let mut staging = None;
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: 1,
                    Height: 1,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut staging),
            )?;
            let staging = required(staging)?;
            self.context.CopySubresourceRegion(
                &staging,
                0,
                0,
                0,
                0,
                &buffer,
                0,
                Some(&D3D11_BOX {
                    left: x,
                    top: y,
                    front: 0,
                    right: x + 1,
                    bottom: y + 1,
                    back: 1,
                }),
            );
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let mut pixel = [0u8; 4];
            pixel.copy_from_slice(std::slice::from_raw_parts(mapped.pData.cast::<u8>(), 4));
            self.context.Unmap(&staging, 0);
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
            Ok(pixel)
        }
    }
    pub fn source_size(&self) -> (u32, u32) {
        self.source_size
    }
    pub fn output_size(&self) -> (u32, u32) {
        self.output_size
    }
    pub fn invalidate(&mut self) {
        self.last_draw = None;
    }
    pub fn configure_cursor(&mut self, scale: f32, opacity: f32, style: u32) {
        self.appearance = [scale, opacity, style as f32];
    }
}

fn shader_error(error: Error, blob: Option<&ID3DBlob>) -> Error {
    if let Some(blob) = blob {
        // SAFETY: compiler blob owns an immutable byte buffer until this function returns.
        let message = unsafe {
            std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
        };
        return Error::new(error.code(), String::from_utf8_lossy(message));
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_cursor_fits_every_edge_at_normal_and_high_dpi_sizes() {
        for style in 0..3 {
            for size in [(100, 100), (640, 480), (12, 8)] {
                for scale in [0.5, 1.0, 6.0, 12.0] {
                    for position in [
                        Point { x: 0.0, y: 0.0 },
                        Point {
                            x: 9999.0,
                            y: 9999.0,
                        },
                    ] {
                        let (position, effective) = fit_cursor(position, size, scale, style);
                        let (left, top, right, bottom) = match style {
                            0 => (1.0, 1.0, 22.0, 35.0),
                            1 => (9.0, 9.0, 9.0, 9.0),
                            _ => (13.0, 13.0, 13.0, 13.0),
                        };
                        assert!(position.x - left * effective >= -0.001);
                        assert!(position.y - top * effective >= -0.001);
                        assert!(position.x + right * effective <= size.0 as f32);
                        assert!(position.y + bottom * effective <= size.1 as f32);
                    }
                }
            }
        }
    }
    #[test]
    fn interior_hotspot_stays_accurate_and_frozen_cursor_survives_source_shrink() {
        assert_eq!(
            fit_cursor(Point { x: 50.0, y: 40.0 }, (100, 100), 1.0, 0),
            (Point { x: 50.0, y: 40.0 }, 1.0)
        );
        assert_eq!(
            fit_cursor(Point { x: 900.0, y: 500.0 }, (640, 480), 1.0, 0).0,
            Point { x: 617.0, y: 444.0 }
        );
    }
}
