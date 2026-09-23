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
const PATCH_FILE: &str = "patches/note-column.patch";

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
    let patch = patches.join(PATCH_FILE).to_string_lossy().into_owned();
    run_command("git", &["apply", &patch], Some(&upstream))?;

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
    fn rejects_every_argument() {
        // 目标版本与模块目录都由 pwsh 决定，没有可调项。
        assert_eq!(main(&["--version".to_owned()]), ExitCode::from(2));
        assert_eq!(main(&["2.4.5".to_owned()]), ExitCode::from(2));
    }
}
