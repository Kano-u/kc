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

    #[cfg(not(windows))]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let candidates: &[(&str, &[&str])] = &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
            ("termux-clipboard-set", &[]),
            ("pbcopy", &[]),
        ];
        let mut last_error = None;
        for (program, args) in candidates {
            let mut child = match Command::new(program)
                .args(*args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    last_error = Some(error.to_string());
                    continue;
                }
            };
            if let Some(mut stdin) = child.stdin.take() {
                if stdin.write_all(text.as_bytes()).is_err() {
                    continue;
                }
            }
            match child.wait() {
                Ok(status) if status.success() => return Ok(()),
                Ok(status) => last_error = Some(status.to_string()),
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        Err(match last_error {
            Some(error) => format!("复制失败: {error}"),
            None => "复制失败: 未找到可用的剪贴板工具。".to_owned(),
        })
    }
}
