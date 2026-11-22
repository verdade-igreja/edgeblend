use anyhow::Result;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Dxgi::*;

pub struct Display {
    pub name: String,
    pub rect: RECT,
    pub adapter: IDXGIAdapter1,
    pub adapter_name: String,
    pub adapter_luid: i64,
    pub output: IDXGIOutput,
}

impl Display {
    pub fn width(&self) -> u32 {
        (self.rect.right - self.rect.left) as u32
    }
    pub fn height(&self) -> u32 {
        (self.rect.bottom - self.rect.top) as u32
    }
}

/// Every display output on every GPU, as DXGI sees them.
pub fn enumerate() -> Result<Vec<Display>> {
    let mut list = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            let ad = adapter.GetDesc1()?;
            let adapter_name = wstr(&ad.Description);
            let luid = ((ad.AdapterLuid.HighPart as i64) << 32) | ad.AdapterLuid.LowPart as i64;
            let mut j = 0;
            while let Ok(output) = adapter.EnumOutputs(j) {
                let d = output.GetDesc()?;
                list.push(Display {
                    name: wstr(&d.DeviceName),
                    rect: d.DesktopCoordinates,
                    adapter: adapter.clone(),
                    adapter_name: adapter_name.clone(),
                    adapter_luid: luid,
                    output,
                });
                j += 1;
            }
            i += 1;
        }
    }
    Ok(list)
}

fn wstr(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}
