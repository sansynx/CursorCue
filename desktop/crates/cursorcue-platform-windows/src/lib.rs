#![cfg(windows)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use windows::{
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D11::{ID3D11Device, ID3D11Texture2D},
            Dxgi::IDXGIDevice,
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
        },
    },
    core::{Interface, Result, factory},
};

pub struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    device: IDirect3DDevice,
    pub item: GraphicsCaptureItem,
    size: windows::Graphics::SizeInt32,
    frames_arrived: Arc<AtomicU64>,
    frame_ready: Arc<AtomicBool>,
    frame_token: i64,
}
pub struct CapturedFrame {
    frame: Direct3D11CaptureFrame,
    pub texture: ID3D11Texture2D,
}
impl Drop for CapturedFrame {
    fn drop(&mut self) {
        let _ = self.frame.Close();
    }
}
impl Capture {
    pub fn new(device: &ID3D11Device, hwnd: HWND) -> Result<Self> {
        // SAFETY: HWND is explicitly selected by the user; the COM device stays alive for the session lifetime.
        let (item, device): (GraphicsCaptureItem, IDirect3DDevice) = unsafe {
            let interop: IGraphicsCaptureItemInterop =
                factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            (
                interop.CreateForWindow(hwnd)?,
                CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)?.cast()?,
            )
        };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let frames_arrived = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&frames_arrived);
        let frame_ready = Arc::new(AtomicBool::new(false));
        let ready = Arc::clone(&frame_ready);
        let frame_token =
            pool.FrameArrived(&windows::Foundation::TypedEventHandler::new(move |_, _| {
                observed.fetch_add(1, Ordering::Relaxed);
                ready.store(true, Ordering::Release);
                Ok(())
            }))?;
        if std::env::args().any(|arg| arg == "--diagnostic") {
            println!(
                "Diagnostic capture: initial pool {}x{}",
                size.Width, size.Height
            );
        }
        let session = pool.CreateCaptureSession(&item)?;
        session.SetIsCursorCaptureEnabled(false)?;
        session.StartCapture()?;
        Ok(Self {
            pool,
            session,
            device,
            item,
            size,
            frames_arrived,
            frame_ready,
            frame_token,
        })
    }
    pub fn cursor_excluded(&self) -> Result<bool> {
        Ok(!self.session.IsCursorCaptureEnabled()?)
    }
    pub fn frames_arrived(&self) -> u64 {
        self.frames_arrived.load(Ordering::Relaxed)
    }
    pub fn latest(&mut self) -> Result<Option<CapturedFrame>> {
        // Clear before draining so a simultaneous arrival remains pending for the next render tick.
        if !self.frame_ready.swap(false, Ordering::AcqRel) {
            return Ok(None);
        }
        let mut latest = None;
        // The pool has two buffers. Drain both so stale frames cannot accumulate.
        for _ in 0..2 {
            let frame = match self.pool.TryGetNextFrame() {
                Ok(frame) => frame,
                // WinRT represents an empty frame-pool result as a null interface.
                Err(error)
                    if error.code().0 == 0
                        || error.code() == windows::Win32::Foundation::E_POINTER =>
                {
                    break;
                }
                Err(error) => return Err(error),
            };
            let size = frame.ContentSize()?;
            if size.Width > 0 && size.Height > 0 && size != self.size {
                if std::env::args().any(|arg| arg == "--diagnostic") {
                    println!(
                        "Diagnostic capture: recreate {}x{} -> {}x{}",
                        self.size.Width, self.size.Height, size.Width, size.Height
                    );
                }
                frame.Close()?;
                drop(latest.take());
                self.size = size;
                self.pool.Recreate(
                    &self.device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    2,
                    size,
                )?;
                return Ok(None);
            }
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            // SAFETY: the interface returns a referenced GPU texture. It is kept alive by the returned COM pointer.
            drop(latest.take());
            latest = Some(CapturedFrame {
                frame,
                texture: unsafe { access.GetInterface::<ID3D11Texture2D>()? },
            });
        }
        Ok(latest)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let diagnostic = std::env::args().any(|arg| arg == "--diagnostic");
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        if diagnostic {
            println!("Diagnostic teardown: before session.Close");
        }
        let _ = self.session.Close();
        if diagnostic {
            println!("Diagnostic teardown: after session.Close; before pool.Close");
        }
        let _ = self.pool.Close();
        if diagnostic {
            println!("Diagnostic teardown: after pool.Close");
        }
    }
}
