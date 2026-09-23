use crate::args;
use crate::data_paths::psreadline_build_dir;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "kc psreadline-patch";
const ALLOWED: &[&str] = &[];

/// 上游 PSReadLine。标签形如 `v2.4.5`，与 PowerShell 自带的版本号一一对应。
const UPSTREAM: &str = "https://github.com/PowerShell/PSReadLine.git";
/// 补丁单独一个仓库：PSReadLine 的补丁与 kc 的版本演进互不相干。
const PATCH_REPO: &str = "https://github.com/Kano-u/PSReadLine-Patch.git";
/// 按顺序应用。每个补丁各自独立、互不重叠，所以能叠在同一份上游源码上。
const PATCH_FILES: &[&str] = &[
    "patches/note-column.patch",
    "patches/prediction-selection.patch",
];

/// 问本机 pwsh：要打补丁的 PSReadLine 版本，以及用户模块目录。
/// 用 `Import-Module` 而不是 `Get-Module -ListAvailable`，因为要的就是启动时真正会加载的那一份。
const QUERY: &str = "Import-Module PSReadLine -ErrorAction Stop; \
    (Get-Module PSReadLine).Version.ToString(); \
    ($env:PSModulePath -split [IO.Path]::PathSeparator)[0]";

/// 装完就地验证：新进程里必须真的加载到刚装的那份。
const VERIFY: &str = "Import-Module PSReadLine -Force -ErrorAction Stop; \
    (Get-Module PSReadLine).Path";

struct Target {
    version: String,
    module_dir: PathBuf,
}

pub fn main(args: &[String]) -> ExitCode {
    if let Err(error) = args::parse(args, ALLOWED) {
        eprintln!("{error}\n用法: {USAGE}");
        return ExitCode::from(2);
    }
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("kc psreadline-patch: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let target = query()?;
    let work = psreadline_build_dir()?;
    reset(&work)?;

    let upstream = work.join("PSReadLine");
    let patches = work.join("patch");
    let upstream_arg = upstream.to_string_lossy().into_owned();
    let patches_arg = patches.to_string_lossy().into_owned();
    let tag = format!("v{}", target.version);

    eprintln!("克隆上游 PSReadLine {tag} …");
    run_command(
        "git",
        &["clone", "--depth", "1", "--branch", &tag, UPSTREAM, &upstream_arg],
        None,
    )?;

    eprintln!("下载补丁 …");
    run_command(
        "git",
        &["clone", "--depth", "1", PATCH_REPO, &patches_arg],
        None,
    )?;

    eprintln!("应用补丁 …");
    for file in PATCH_FILES {
        let patch = patches.join(file).to_string_lossy().into_owned();
        run_command("git", &["apply", &patch], Some(&upstream))?;
    }

    eprintln!("编译（约一分钟）…");
    publish(&upstream)?;
    polyfill(&upstream)?;

    let dest = target.module_dir.join("PSReadLine").join(&target.version);
    install(&upstream, &dest)?;

    let loaded = verify()?;
    if !Path::new(&loaded).starts_with(&dest) {
        return Err(format!(
            "装好的补丁没有生效：pwsh 实际加载的是 {loaded}。\
             用户模块目录里可能存在版本更高的 PSReadLine，先删掉它再重试。"
        ));
    }

    Ok(format!(
        "已给 PSReadLine {} 打上补丁，模块目录 {}\n当前 PowerShell 窗口要重启才会加载。",
        target.version,
        dest.display()
    ))
}

fn query() -> Result<Target, String> {
    let output = Command::new("pwsh")
        .args(["-NoProfile", "-Command", QUERY])
        .output()
        .map_err(|error| format!("无法执行 pwsh: {error}；本命令需要 PowerShell 7"))?;
    if !output.status.success() {
        return Err(format!(
            "pwsh 查询 PSReadLine 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_target(&String::from_utf8_lossy(&output.stdout))
}

fn parse_target(stdout: &str) -> Result<Target, String> {
    let mut lines = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let version = lines
        .next()
        .ok_or_else(|| "pwsh 没有报告 PSReadLine 版本".to_owned())?;
    let module_dir = lines
        .next()
        .ok_or_else(|| "pwsh 没有报告用户模块目录".to_owned())?;
    Ok(Target {
        version: version.to_owned(),
        module_dir: PathBuf::from(module_dir),
    })
}

fn verify() -> Result<String, String> {
    let output = Command::new("pwsh")
        .args(["-NoProfile", "-Command", VERIFY])
        .output()
        .map_err(|error| format!("无法执行 pwsh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "补丁版 PSReadLine 导入失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// 主模块按 netstandard2.0 发布；官方 `LayoutModule` 也是取这个产物。
fn publish(upstream: &Path) -> Result<(), String> {
    let project = upstream.join("PSReadLine").join("PSReadLine.csproj");
    run_command(
        "dotnet",
        &[
            "publish",
            "-c",
            "Release",
            "-f",
            "netstandard2.0",
            "--nologo",
            &project.to_string_lossy(),
        ],
        None,
    )
}

/// Polyfiller 要两个目标框架：net6plus 给 PowerShell 6+，netstd 给 5.1。
/// 少了它，`PSConsoleReadLine` 的静态构造会直接抛异常。
fn polyfill(upstream: &Path) -> Result<(), String> {
    let project = upstream.join("Polyfill").join("Polyfill.csproj");
    run_command(
        "dotnet",
        &["build", "-c", "Release", "--nologo", &project.to_string_lossy()],
        None,
    )
}

/// 模块目录布局照抄官方 `LayoutModule`：少一个文件都会在导入时炸。
fn install(upstream: &Path, dest: &Path) -> Result<(), String> {
    // 删之前先探一遍：删到一半才撞上文件锁，会留下一个半残的模块目录。
    ensure_replaceable(dest)?;
    remove(dest)?;
    let net6plus = dest.join("net6plus");
    let netstd = dest.join("netstd");
    std::fs::create_dir_all(&net6plus).map_err(|error| write_error(dest, &error))?;
    std::fs::create_dir_all(&netstd).map_err(|error| write_error(dest, &error))?;

    let publish = upstream.join("PSReadLine/bin/Release/netstandard2.0/publish");
    let source = upstream.join("PSReadLine");
    let polyfill = upstream.join("Polyfill/bin/Release");

    let files = [
        (publish.join("Microsoft.PowerShell.PSReadLine.dll"), dest.join("Microsoft.PowerShell.PSReadLine.dll")),
        (publish.join("Microsoft.PowerShell.Pager.dll"), dest.join("Microsoft.PowerShell.Pager.dll")),
        (source.join("PSReadLine.psm1"), dest.join("PSReadLine.psm1")),
        (source.join("PSReadLine.psd1"), dest.join("PSReadLine.psd1")),
        (source.join("PSReadLine.format.ps1xml"), dest.join("PSReadLine.format.ps1xml")),
        (polyfill.join("net6.0/Microsoft.PowerShell.PSReadLine.Polyfiller.dll"), net6plus.join("Microsoft.PowerShell.PSReadLine.Polyfiller.dll")),
        (polyfill.join("netstandard2.0/Microsoft.PowerShell.PSReadLine.Polyfiller.dll"), netstd.join("Microsoft.PowerShell.PSReadLine.Polyfiller.dll")),
    ];
    for (from, to) in files {
        std::fs::copy(&from, &to)
            .map_err(|error| format!("复制 {} 到 {} 失败: {error}", from.display(), to.display()))?;
    }
    Ok(())
}

/// Windows 上已被 pwsh 加载的 DLL 既不能覆盖也不能删除。装之前先探一遍，
/// 有任一文件被占用就整体拒绝 —— 否则 `remove` 会删掉一部分文件后中止，
/// 留下缺 `Pager.dll` 之类的半残模块目录。
fn ensure_replaceable(dest: &Path) -> Result<(), String> {
    if !dest.exists() {
        return Ok(());
    }
    let mut locked = Vec::new();
    collect_locked(dest, &mut locked)?;
    if locked.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} 下的 {} 个文件正被其他进程占用。先关闭所有 PowerShell 窗口（含运行中的 pwsh 会话）再重试：\n{}",
        dest.display(),
        locked.len(),
        locked
            .iter()
            .map(|path| format!("  {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn collect_locked(dir: &Path, locked: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|error| write_error(dir, &error))?;
    for entry in entries {
        let path = entry
            .map_err(|error| write_error(dir, &error))?
            .path();
        if path.is_dir() {
            collect_locked(&path, locked)?;
        } else if is_locked(&path) {
            locked.push(path);
        }
    }
    Ok(())
}

/// 以写方式打开探测：被加载的程序集只会以共享读方式打开，写打开必然失败。
/// 不删文件、不写内容，探测本身无副作用。
fn is_locked(path: &Path) -> bool {
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(file) => {
            drop(file);
            false
        }
        // 权限问题也算“不能替换”：反正接下来也删不掉。
        Err(_) => true,
    }
}

/// Windows 上已被 pwsh 加载的 DLL 会被锁住，覆盖必然失败；这种情况要给出可操作的提示。
fn write_error(dest: &Path, error: &std::io::Error) -> String {
    format!(
        "写入 {} 失败: {error}。若已有 PowerShell 窗口正在使用这份模块，全部关闭后重试。",
        dest.display()
    )
}

fn reset(work: &Path) -> Result<(), String> {
    remove(work)?;
    std::fs::create_dir_all(work).map_err(|error| {
        format!("创建构建目录 {} 失败: {error}", work.display())
    })
}

fn remove(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(path)
        .map_err(|error| write_error(path, &error))
}

/// 外部命令统一从这里走。输出直接继承，编译进度能实时看到；
/// 失败时带上完整命令，便于区分是 git、dotnet 还是 pwsh 的问题。
fn run_command(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let status = command
        .status()
        .map_err(|error| format!("无法执行 {program}: {error}"))?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "{program} {} 失败（退出码 {:?}）",
        args.join(" "),
        status.code()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_and_the_module_directory() {
        let target = parse_target("2.4.5\r\nC:\\Users\\k\\Documents\\PowerShell\\Modules\r\n").unwrap();
        assert_eq!(target.version, "2.4.5");
        assert_eq!(
            target.module_dir,
            PathBuf::from(r"C:\Users\k\Documents\PowerShell\Modules")
        );
    }

    #[test]
    fn ignores_blank_lines_from_the_powershell_output() {
        let target = parse_target("\n2.4.5\n\n/tmp/modules\n").unwrap();
        assert_eq!(target.version, "2.4.5");
        assert_eq!(target.module_dir, PathBuf::from("/tmp/modules"));
    }

    #[test]
    fn a_short_answer_is_an_error_not_a_guess() {
        assert!(parse_target("").is_err());
        assert!(parse_target("2.4.5\n").is_err());
    }

    #[test]
    fn applies_every_patch_in_the_repository() {
        // 每个补丁各自独立、互不重叠，必须全部应用；漏掉一个就会静默丢功能。
        assert!(PATCH_FILES.contains(&"patches/note-column.patch"));
        assert!(PATCH_FILES.contains(&"patches/prediction-selection.patch"));
    }

    #[test]
    fn detects_no_lock_on_a_writable_directory() {
        // 探测本身不能留下副作用，也不能误报普通文件。
        let dir = std::env::temp_dir().join(format!("kc-patch-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("net6plus")).unwrap();
        std::fs::write(dir.join("a.dll"), b"a").unwrap();
        std::fs::write(dir.join("net6plus/b.dll"), b"b").unwrap();

        assert!(!is_locked(&dir.join("a.dll")));
        assert!(ensure_replaceable(&dir).is_ok());
        assert!(ensure_replaceable(&dir.join("missing")).is_ok());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_every_argument() {
        // 目标版本与模块目录都由 pwsh 决定，没有可调项。
        assert_eq!(main(&["--version".to_owned()]), ExitCode::from(2));
        assert_eq!(main(&["2.4.5".to_owned()]), ExitCode::from(2));
    }
}
