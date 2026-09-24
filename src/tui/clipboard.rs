pub(super) fn copy_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::iter;
        use std::ptr;
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        use windows_sys::Win32::System::Memory::{
            GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
        };
        use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

        let text = text.replace('\n', "\r\n");
        let wide: Vec<u16> = text.encode_utf16().chain(iter::once(0)).collect();
        let bytes = wide
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .ok_or_else(|| "命令过长，无法复制。".to_owned())?;

        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if handle.is_null() {
                return Err("无法分配剪贴板内存。".to_owned());
            }
            let pointer = GlobalLock(handle).cast::<u16>();
            if pointer.is_null() {
                return Err("无法写入剪贴板。".to_owned());
            }
            ptr::copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
            let _ = GlobalUnlock(handle);

            if OpenClipboard(ptr::null_mut()) == 0 {
                return Err("无法打开系统剪贴板。".to_owned());
            }
            if EmptyClipboard() == 0 {
                let _ = CloseClipboard();
                return Err("无法清空系统剪贴板。".to_owned());
            }
            if SetClipboardData(u32::from(CF_UNICODETEXT), handle).is_null() {
                let _ = CloseClipboard();
                return Err("无法写入系统剪贴板。".to_owned());
            }
            let _ = CloseClipboard();
        }
        Ok(())
    }

    // Termux 是唯一的非 Windows 目标，只认这一个命令，不装就报错。
    #[cfg(not(windows))]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        const PROGRAM: &str = "termux-clipboard-set";
        let mut child = Command::new(PROGRAM)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("复制失败: 无法启动 {PROGRAM}: {error}"))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("复制失败: 无法写入 {PROGRAM}。"))?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|error| format!("复制失败: 写入 {PROGRAM} 出错: {error}"))?;
        drop(stdin);

        let status = child
            .wait()
            .map_err(|error| format!("复制失败: 等待 {PROGRAM} 出错: {error}"))?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("复制失败: {PROGRAM} 以 {status} 退出。"))
    }
}
