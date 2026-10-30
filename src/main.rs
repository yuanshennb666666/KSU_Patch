//! ksupatch -- offline KernelSU LKM boot-image patcher.
//!
//! Patches a GKI boot / init_boot image with KernelSU in LKM mode, using only the payload
//! that the official KernelSU APK carries inside its own native library. No device, no
//! adb, no root, no network.

mod assets;
mod axml;
mod boot;
mod elf;
mod err;
mod sha1;
mod style;
mod ui;
mod zip;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use err::{fail, report, Res, APK, ASSET, INPUT, IO, KMI, USAGE};

/// Colour the section headings of the help text.
fn color_help(text: &str) -> String {
    text.lines()
        .map(|line| {
            if !line.starts_with(' ') && line.ends_with(':') {
                style::bold_cyan(line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::from(err::OK),
        Err(f) => {
            report(&f);
            ExitCode::from(f.exit)
        }
    }
}

fn usage() -> String {
    format!(
        "\
ksupatch {}
Offline KernelSU LKM boot-image patcher.

Usage:
  ksupatch <manager.apk> <image> [kmi] [options]
  ksupatch --dump-assets <manager.apk>
  ksupatch --version

Arguments:
  <manager.apk>         KernelSU-family manager APK: official, KernelSU-Next,
                        SukiSU-Ultra, MakoSU, 7kimisu, ... The file name does not
                        matter; the package and embedded assets identify it.
  <image>               boot.img, init_boot.img or vendor_boot.img (GKI v3/v4).
  [kmi]                 Full KMI (android14-6.1), kernel version (6.6), or the
                        full `uname -r` release (5.10.198-android13-8-g...).
                        Default: auto.

Options:
  -o, --output <path>   Output file, or directory ending in a separator.
      --out <dir>       Output directory.
      --out-name <name> Output file name.
      --module <path>   Use this kernelsu.ko/sevenk.ko instead of the APK's.
      --arch <arch>     aarch64 (default), x86_64 or riscv64; auto-detected.
      --kmi-from <img>  Read the kernel version from this image.
      --ksu-config <t>  Write ksu_config with these tokens.
      --no-stock-backup Do not write stock_image.sha1.
      --verify          Re-parse the output and check the install.
  -v, --verbose         Show the detailed eight-step trace.
      --dump-assets     List the APK assets and exit.
      --color <when>    auto (default), always or never; NO_COLOR is honoured.
  -h, --help            Print help.
  -V, --version         Print version.

KMI detection:
  --kmi-from <image> is an explicit override: when given, it is read first and
  must agree with any explicit KMI. Otherwise auto uses, in order:
  1. the kernel inside the image (boot.img)
  2. the vermagic of a module already in the ramdisk
  3. a sibling boot.img next to the image (exact name)
  init_boot and vendor_boot carry no kernel; tell the KMI if none applies.
  A KMI follows the kernel branch, not the Android version in build.prop.

Exit codes:
  0 ok   1 usage   2 input   3 apk   4 image
  5 kmi  6 asset   7 inject  8 io

Examples:
  ksupatch manager.apk init_boot.img android14-6.1 --verify
  ksupatch SukiSU.apk boot.img auto --out out\\
",
        env!("CARGO_PKG_VERSION")
    )
}

#[derive(Default)]
struct Args {
    positional: Vec<String>,
    out: Option<String>,
    out_name: Option<String>,
    module: Option<String>,
    arch: Option<String>,
    verify: bool,
    dump_assets: bool,
    verbose: bool,
    ksu_config: Option<String>,
    no_stock_backup: bool,
    kmi_from: Option<String>,
    output: Option<String>,
    color: Option<String>,
}

fn parse_args(argv: &[String]) -> Res<Args> {
    let mut a = Args::default();
    let mut i = 0usize;
    while i < argv.len() {
        let arg = argv[i].as_str();
        if let Some(v) = arg.strip_prefix("--color=") {
            a.color = Some(v.to_string());
            i += 1;
            continue;
        }
        let mut take = |name: &str| -> Res<String> {
            if i + 1 >= argv.len() {
                return Err(fail(
                    USAGE,
                    "argument parsing",
                    name.to_string(),
                    format!("`{name}` needs a value"),
                    "run `ksupatch --help`",
                ));
            }
            i += 1;
            Ok(argv[i].clone())
        };
        match arg {
            "--out" => a.out = Some(take("--out")?),
            "--out-name" => a.out_name = Some(take("--out-name")?),
            "--module" => a.module = Some(take("--module")?),
            "--arch" => a.arch = Some(take("--arch")?),
            "--verify" => a.verify = true,
            "--verbose" | "-v" => a.verbose = true,
            "--dump-assets" => a.dump_assets = true,
            "--ksu-config" => a.ksu_config = Some(take("--ksu-config")?),
            "--no-stock-backup" => a.no_stock_backup = true,
            "--kmi-from" => a.kmi_from = Some(take("--kmi-from")?),
            "--output" | "-o" => a.output = Some(take("--output")?),
            "--color" => a.color = Some(take("--color")?),
            other if other.starts_with("--") => {
                return Err(fail(
                    USAGE,
                    "argument parsing",
                    argv.join(" "),
                    format!("`{other}` is not a known option"),
                    "expected: ksupatch <manager.apk> <boot.img|init_boot.img> [<kmi>|auto] \
                     [options] -- run `ksupatch --help`",
                ));
            }
            other => a.positional.push(other.to_string()),
        }
        i += 1;
    }
    Ok(a)
}

fn run(argv: &[String]) -> Res<()> {
    if argv.is_empty() || argv.iter().any(|a| a == "--help" || a == "-h") {
        let _ = style::init(style::Choice::Auto);
        println!("{}", color_help(&usage()));
        return Ok(());
    }
    if argv.iter().any(|a| a == "--version" || a == "-V") {
        let _ = style::init(style::Choice::Auto);
        println!(
            "{} {}",
            style::bold_cyan("ksupatch"),
            style::dim(env!("CARGO_PKG_VERSION"))
        );
        return Ok(());
    }
    let args = parse_args(argv)?;

    let choice = match args.color.as_deref() {
        None | Some("auto") => style::Choice::Auto,
        Some("always") => style::Choice::Always,
        Some("never") => style::Choice::Never,
        Some(other) => {
            return Err(fail(
                USAGE,
                "argument parsing",
                other.to_string(),
                format!("unknown --color value `{other}`"),
                "use auto, always or never",
            ))
        }
    };
    let _ = style::init(choice);

    if args.dump_assets {
        let Some(apk) = args.positional.first() else {
            return Err(fail(
                USAGE,
                "argument parsing",
                "(none)",
                "--dump-assets requires an APK path",
                "ksupatch --dump-assets <manager.apk>",
            ));
        };
        return dump_assets(Path::new(apk));
    }

    match args.positional.len() {
        1 => dump_assets(Path::new(&args.positional[0])),
        2 | 3 => patch(&args),
        n => Err(fail(
            USAGE,
            "argument parsing",
            argv.join(" "),
            format!("expected 2 or 3 positional arguments, got {n}"),
            "ksupatch <manager.apk> <boot.img|init_boot.img> [<kmi>|auto]",
        )),
    }
}

// =====================================================================================
// Asset inspection (also the self-contained verification entry point)
// =====================================================================================

struct Scanned {
    name: String,
    csize: u64,
    lib_len: usize,
    ranges: Vec<(usize, usize)>,
    found: Vec<assets::Asset>,
    supported: Vec<String>,
    /// Ramdisk filename the manager's init expects (`kernelsu.ko` or `sevenk.ko`).
    module_file: &'static str,
    extract_secs: f64,
    scan_secs: f64,
}

fn dump_assets(apk: &Path) -> Res<()> {
    ui::set_verbose(true);
    let started = Instant::now();
    let apk = apk.to_path_buf();
    let meta = std::fs::metadata(&apk).map_err(|e| {
        fail(
            INPUT,
            "input validation",
            apk.display().to_string(),
            format!("cannot stat file: {e}"),
            "check the path; relative paths resolve against the current directory",
        )
    })?;
    if !meta.is_file() {
        return Err(fail(
            INPUT,
            "input validation",
            apk.display().to_string(),
            "path is not a regular file",
            "pass the KernelSU-family Manager .apk file itself",
        ));
    }

    ui::header(env!("CARGO_PKG_VERSION"), "APK inspection");
    ui::field("APK", apk.display());
    ui::field("Size", ui::bytes(meta.len() as usize));

    let zip = zip::Zip::open(&apk).map_err(|e| {
        fail(
            APK,
            "APK container",
            apk.display().to_string(),
            e,
            "this expects an unmodified KernelSU release .apk (a ZIP container)",
        )
    })?;
    ui::field("ZIP entries", zip.entries.len());
    print_manifest(&zip);

    let libs = find_libs(&zip, &apk)?;
    ui::field(
        "ABIs",
        format!(
            "{} ({})",
            libs.len(),
            libs.iter()
                .map(|e| e.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );

    println!();
    println!("{}", style::bold_cyan("Extract and scan"));
    let mut scans: Vec<Scanned> = Vec::new();
    for entry in &libs {
        let s = scan_lib(&zip, entry)?;
        println!(
            "  {:<30} {:>3} assets   {} -> {}   ({:.2}s + {:.2}s)",
            s.name,
            s.found.len(),
            ui::size(s.csize as usize),
            ui::size(s.lib_len),
            s.extract_secs,
            s.scan_secs
        );
        scans.push(s);
    }

    let mut seed: Vec<(String, String)> = Vec::new();
    for s in &scans {
        for pair in assets::ddk_anchors(&s.found) {
            if !seed.contains(&pair) {
                seed.push(pair);
            }
        }
    }
    if !seed.is_empty() {
        println!();
        println!("{}", style::bold_cyan("KMI anchors from DDK paths"));
        for (android, clang) in &seed {
            println!("  {android} <-> clang {clang}");
        }
    }

    for s in scans.iter_mut() {
        report_lib(s, &seed);
    }

    let abis = scans.len();
    let assets: usize = scans.iter().map(|s| s.found.len()).sum();
    let kmis = scans.iter().map(|s| s.supported.len()).max().unwrap_or(0);
    let helpers = scans
        .iter()
        .flat_map(|s| s.found.iter())
        .filter(|a| a.kind == assets::Kind::Helper)
        .count();
    let auxiliary = scans
        .iter()
        .flat_map(|s| s.found.iter())
        .filter(|a| a.kind == assets::Kind::AuxModule)
        .count();
    ui::done(
        "INSPECTION COMPLETE",
        &[
            ("ABIs", abis.to_string()),
            ("Assets", assets.to_string()),
            ("KMIs", kmis.to_string()),
            ("Helpers", helpers.to_string()),
            ("Auxiliary", auxiliary.to_string()),
            ("Time", format!("{:.2}s", started.elapsed().as_secs_f64())),
        ],
    );
    Ok(())
}

fn print_manifest(zip: &zip::Zip) {
    match zip.find("AndroidManifest.xml") {
        Some(e) => match zip.read(e).and_then(|b| axml::parse(&b)) {
            Ok(m) => {
                ui::field(
                    "Manager",
                    style::bold_cyan(manager_name(m.package.as_deref().unwrap_or(""))),
                );
                ui::field("Package", m.package.as_deref().unwrap_or("(not found)"));
                ui::field(
                    "Version",
                    format!(
                        "{} ({})",
                        m.version_name.as_deref().unwrap_or("(not found)"),
                        m.version_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "?".to_string())
                    ),
                );
            }
            Err(e) => ui::field("Manifest", format!("could not be read ({e})")),
        },
        None => ui::field("Manifest", "AndroidManifest.xml is absent"),
    }
}

/// Human name for the Manager package this APK belongs to.
///
/// The APK's file name is irrelevant: this is the only identification the tool uses, plus
/// the embedded payload itself when the package is not one of the known managers.
fn manager_name(package: &str) -> &'static str {
    match package {
        "me.weishu.kernelsu" => "KernelSU (official)",
        "com.rifsxd.ksunext" => "KernelSU-Next",
        "com.sukisu.ultra" => "SukiSU-Ultra",
        "com.sukisu.sukisu" => "SukiSU (legacy)",
        "com.resukisu.resukisu" => "ReSukiSU / BakaSU",
        "iqknto.ohlwqk" => "KittiSU",
        "com.xecpro.kernel" => "XECKernelPro",
        "com.qrj.apexsu" => "ApexSU",
        "com.makosu.manager" => "MakoSU (MKSU)",
        "com.sevenk.core" => "7kimisu (7KSU)",
        "com.topjohnwu.magisk" => "Magisk (not a KernelSU manager)",
        "me.bmax.apatch" => "APatch (not a KernelSU manager)",
        "" => "(unknown)",
        _ => "unrecognised package",
    }
}

/// ABIs that can carry a KernelSU LKM payload. The kernel side is GKI-only, so a fork's
/// 32-bit container is not a patch target even when it embeds assets of its own.
fn is_payload_abi(name: &str) -> bool {
    name.starts_with("lib/arm64-v8a/")
        || name.starts_with("lib/x86_64/")
        || name.starts_with("lib/riscv64/")
}

/// Native libraries that carry the KernelSU payload.
///
/// Upstream and KernelSU-Next name it `lib/<abi>/libksud.so`. SukiSU ships the same
/// container under an obfuscated name, so the canonical name is a fast path rather than a
/// requirement: when it is absent, every 64-bit `lib/**/*.so` is probed for the embedded
/// asset name table (`_kernelsu.ko`, or `_sevenk.ko` for 7kimisu), which no other library
/// carries.
fn find_libs(zip: &zip::Zip, apk: &Path) -> Res<Vec<zip::Entry>> {
    let all: Vec<zip::Entry> = zip
        .entries
        .iter()
        .filter(|e| e.name.starts_with("lib/") && e.name.ends_with(".so"))
        .cloned()
        .collect();
    if all.is_empty() {
        return Err(fail(
            APK,
            "APK contents",
            apk.display().to_string(),
            "no `lib/<abi>/*.so` entry found, so this APK carries no KernelSU payload",
            "make sure this is a KernelSU-family Manager APK (official, KernelSU-Next, SukiSU, ...)",
        ));
    }
    let libs: Vec<zip::Entry> = all
        .iter()
        .filter(|e| is_payload_abi(&e.name))
        .cloned()
        .collect();
    if libs.is_empty() {
        return Err(fail(
            APK,
            "APK contents",
            apk.display().to_string(),
            "this APK has no 64-bit native library (arm64-v8a, x86_64 or riscv64), which the \
             KernelSU LKM payload requires",
            format!(
                "found only: {}",
                all.iter()
                    .map(|e| e.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    let canonical: Vec<zip::Entry> = libs
        .iter()
        .filter(|e| e.name.ends_with("/libksud.so"))
        .cloned()
        .collect();
    if !canonical.is_empty() {
        return Ok(canonical);
    }

    let mut payload = Vec::new();
    for entry in &libs {
        let Ok(data) = zip.read(entry) else { continue };
        if elf::contains(&data, b"_kernelsu.ko") || elf::contains(&data, b"_sevenk.ko") {
            payload.push(entry.clone());
        }
    }
    if payload.is_empty() {
        return Err(fail(
            APK,
            "APK contents",
            apk.display().to_string(),
            "no native library carries a KernelSU module name table (`_kernelsu.ko` / \
             `_sevenk.ko`)",
            "this does not look like a KernelSU-family APK that embeds LKM assets",
        ));
    }
    Ok(payload)
}

fn scan_lib(zip: &zip::Zip, entry: &zip::Entry) -> Res<Scanned> {
    let label = entry.name.clone();
    let t = Instant::now();
    let lib = zip.read(entry).map_err(|e| {
        fail(
            APK,
            "APK extraction",
            label.clone(),
            e,
            "the APK may be truncated or corrupted; re-download it",
        )
    })?;
    let extract_secs = t.elapsed().as_secs_f64();
    if lib.len() as u64 != entry.usize_ {
        return Err(fail(
            ASSET,
            "APK integrity",
            label,
            format!(
                "inflated size {} does not match the size declared in the zip directory ({})",
                lib.len(),
                entry.usize_
            ),
            "the APK is corrupted; re-download it",
        ));
    }

    let ranges = scan_ranges(&lib);
    let t = Instant::now();
    let found = assets::scan_ranges(&lib, &ranges);
    let scan_secs = t.elapsed().as_secs_f64();

    Ok(Scanned {
        name: entry.name.clone(),
        csize: entry.csize,
        lib_len: lib.len(),
        ranges,
        supported: assets::supported_kmi(&lib),
        module_file: assets::module_file_name(&lib),
        found,
        extract_secs,
        scan_secs,
    })
}

fn report_lib(s: &mut Scanned, seed: &[(String, String)]) {
    println!();
    println!("{}", style::bold_cyan(&s.name));
    ui::field("Extracted", ui::bytes(s.lib_len));
    let total: usize = s.ranges.iter().map(|(a, b)| b - a).sum();
    ui::field(
        "Scanned",
        format!(
            "{} in {} range(s): {}",
            ui::bytes(total),
            s.ranges.len(),
            s.ranges
                .iter()
                .map(|(a, b)| format!("{a:#X}..{b:#X}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    ui::field("Assets", s.found.len());
    ui::field(
        "KMIs",
        if s.supported.is_empty() {
            style::dim("none").to_string()
        } else {
            style::bold_yellow(&s.supported.join(", "))
        },
    );
    println!();
    println!(
        "  {}",
        style::dim(&format!(
            "{:<3} {:<10} {:<10} {:<10} {:<8} {:<8} {}",
            "#", "Offset", "CSize", "USize", "Arch", "Type", "Identity"
        ))
    );
    for (i, a) in s.found.iter().enumerate() {
        println!(
            "  {:<3} {:<10} {:<10} {:<10} {:<8} {:<8} {}",
            i + 1,
            format!("{:#09X}", a.offset),
            group(a.csize),
            group(a.data.len()),
            a.arch(),
            a.type_name(),
            a.identity()
        );
    }
    println!();

    let aux = s
        .found
        .iter()
        .filter(|a| a.kind == assets::Kind::AuxModule)
        .count();
    if aux > 0 {
        ui::field(
            "Auxiliary",
            format!("{aux} modules (shipped alongside the payload; not KMI-selectable)"),
        );
    }

    let judge = assets::judge(&mut s.found, &s.supported, seed);
    println!(
        "{}",
        style::bold_cyan(&format!(
            "Module -> KMI ({} of {})",
            judge.assigned.len(),
            judge.module_count
        ))
    );
    let mut by_kmi: Vec<&(usize, String)> = judge.assigned.iter().collect();
    by_kmi.sort_by(|a, b| a.1.cmp(&b.1));
    for (idx, kmi) in by_kmi {
        let a = &s.found[*idx];
        println!(
            "  {:<16} {:>10}  {}",
            kmi,
            ui::size(a.data.len()),
            a.vermagic.as_deref().unwrap_or("(absent)")
        );
        for line in &a.evidence {
            println!("    {}", style::dim(&format!("evidence: {line}")));
        }
    }
    if !judge.unresolved.is_empty() {
        println!();
        println!("{}", style::bold_yellow("Unresolved modules"));
        for idx in &judge.unresolved {
            let a = &s.found[*idx];
            println!(
                "  {:>10}  vermagic {:<58} ddk {}  clang {}",
                ui::size(a.data.len()),
                a.vermagic.as_deref().unwrap_or("-"),
                a.ddk_kmi.as_deref().unwrap_or("-"),
                a.clang.as_deref().unwrap_or("-")
            );
        }
    }
    self_check(&s.found, &judge, &s.supported);
}

fn self_check(found: &[assets::Asset], judge: &assets::Judge, supported: &[String]) {
    println!();
    println!("{}", style::bold_cyan("Self-check"));
    let mut failed = 0usize;
    let mut check = |pass: bool, what: String| {
        println!(
            "    [{}] {}",
            if pass {
                style::green("ok")
            } else {
                style::bold_red("FAIL")
            },
            what
        );
        if !pass {
            failed += 1;
        }
    };

    check(
        judge.module_count == supported.len(),
        format!(
            "module count ({}) equals the number of declared KMI names ({})",
            judge.module_count,
            supported.len()
        ),
    );
    check(
        found
            .iter()
            .filter(|a| a.kind == assets::Kind::Module)
            .all(|a| a.elf.map(|e| e.etype) == Some(elf::ET_REL)),
        "every kernel module is a well-formed ELF ET_REL object".into(),
    );
    check(
        judge.unresolved.is_empty(),
        format!(
            "every module maps to a unique KMI ({} unresolved)",
            judge.unresolved.len()
        ),
    );
    check(
        judge.assigned.iter().all(|(idx, kmi)| {
            let want = kmi.rsplit('-').next().unwrap_or("");
            found[*idx]
                .vermagic
                .as_deref()
                .and_then(elf::kernel_xy)
                .as_deref()
                == Some(want)
        }),
        "each assigned KMI's kernel version agrees with the module vermagic".into(),
    );
    check(
        {
            let mut v: Vec<&String> = judge.assigned.iter().map(|(_, k)| k).collect();
            let n = v.len();
            v.sort();
            v.dedup();
            v.len() == n
        },
        "no KMI is assigned to more than one module".into(),
    );
    let ksuinit = found
        .iter()
        .filter(|a| a.kind == assets::Kind::KsuInit)
        .count();
    if supported.is_empty() && judge.module_count == 0 {
        // Some forks ship a partial ABI (e.g. MakoSU's x86_64 container has no LKM
        // payload). That ABI is unusable for patching, but it is not a parse failure.
        check(
            true,
            format!("no LKM payload in this ABI (0 modules, {ksuinit} ksuinit)"),
        );
    } else {
        check(
            ksuinit == 1,
            format!("exactly one ksuinit recovered (found {ksuinit})"),
        );
    }
    let unknown = found
        .iter()
        .filter(|a| a.kind == assets::Kind::Unknown)
        .count();
    check(
        unknown == 0,
        format!("every recovered asset was identified (found {unknown} unrecognised)"),
    );

    let total = 7;
    println!(
        "    result: {}",
        if failed == 0 {
            style::bold_green(&format!("PASS ({total} checks)"))
        } else {
            style::bold_red(&format!("FAIL -- {failed} of {total} checks failed"))
        }
    );
}

// =====================================================================================
// Patching
// =====================================================================================

fn abi_of(arch: &str) -> Option<&'static str> {
    match arch {
        "aarch64" => Some("arm64-v8a"),
        "x86_64" => Some("x86_64"),
        "riscv64" => Some("riscv64"),
        _ => None,
    }
}

/// `6.6`, `5.10`, ... -- a bare kernel major.minor, as `uname -r` starts with.
fn is_kernel_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
}

/// Parse what a user may paste as the explicit KMI.
///
/// `uname -r` on a GKI device is a full kernel release, not a KMI:
///
/// ```text
/// 5.10.198-android13-8-g1a2b3c4d5e6f
/// ```
///
/// The KMI is the Android branch plus major.minor (`android13-5.10`).  Accept the bare
/// version, the full release, and the release without an Android token; return the Android
/// branch only when the release actually names one, because a custom kernel often does not.
fn parse_kernel_input(s: &str) -> Option<(String, Option<String>)> {
    let (release, tail) = match s.split_once('-') {
        Some((release, tail)) => (release, Some(tail)),
        None => (s, None),
    };
    let mut parts = release.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    if major.is_empty() || minor.is_empty() {
        return None;
    }
    let numeric = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit());
    if !numeric(major) || !numeric(minor) {
        return None;
    }
    // A full release normally has a patch level (`5.10.198`); accept any number of extra
    // numeric components so a vendor's `5.10.198.1` is not rejected for being unusual.
    if !parts.all(numeric) {
        return None;
    }
    let mut android = None;
    if let Some(tail) = tail {
        for token in tail.split('-') {
            if let Some(digits) = token.strip_prefix("android") {
                // A malformed branch token is a typo, not a localversion: refuse it rather
                // than silently falling back to the ramdisk's build.prop.
                if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                if android.is_none() {
                    android = Some(format!("android{digits}"));
                }
            }
        }
    }
    Some((format!("{major}.{minor}"), android))
}

/// Look through the folder the image came from for the firmware's `boot.img`.
///
/// A firmware package ships `boot.img` next to `init_boot.img`, and the kernel lives in
/// `boot.img`, so reading it is the same evidence `uname -r` would give -- no version to
/// type. Only the canonical name is accepted: a folder can hold images for more than one
/// device, and quietly picking up a sibling by some other name could install a module for
/// the wrong kernel. Anything else has to be named explicitly with --kmi-from.
fn discover_sibling_kernel(image: &Path, expect_arch: Option<&str>) -> Option<(String, String)> {
    // A bare file name has an empty parent, and an empty parent means the current
    // directory -- not "no directory to search".
    let dir = match image.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let sibling = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.is_file()
                && p != image
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.eq_ignore_ascii_case("boot.img"))
                    .unwrap_or(false)
        })?;

    let data = std::fs::read(&sibling).ok()?;
    let name = sibling
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("boot.img")
        .to_string();
    let (arch, kmi, release) = boot::kernel_facts_from_image(&data)?;
    if expect_arch.map(|want| want == arch).unwrap_or(true) {
        if let Some(kmi) = kmi {
            return Some((kmi, name));
        }
        // A custom kernel often has no `androidNN` token in its banner. Its
        // `major.minor` is still enough for the build.prop/platform fallback in
        // resolve_kernel_version(), so do not discard the sibling entirely.
        if let Some(xy) = release.as_deref().and_then(elf::kernel_xy) {
            return Some((xy, name));
        }
    }
    None
}

/// Check a user-supplied kernel module before it is written into the ramdisk.
fn validate_module(data: &[u8], arch: &str, kmi: &str) -> Res<()> {
    let elf = elf::ElfInfo::parse(data).ok_or_else(|| {
        fail(
            ASSET,
            "module validation",
            "--module",
            "the file is not a 64-bit little-endian ELF object",
            "pass a kernel module built for this device, normally kernelsu.ko",
        )
    })?;
    if elf.etype != elf::ET_REL {
        return Err(fail(
            ASSET,
            "module validation",
            "--module",
            format!(
                "the file is an ELF {} object, not a relocatable kernel module (ET_REL)",
                elf.type_name()
            ),
            "pass a kernelsu.ko / sevenk.ko from a KernelSU-family APK",
        ));
    }
    if elf.arch() != arch {
        return Err(fail(
            ASSET,
            "module validation",
            "--module",
            format!(
                "the module is built for {}, but the image target is {arch}",
                elf.arch()
            ),
            "use the matching ABI, or pass the right --arch",
        ));
    }
    if let Some(ddk) = elf::ddk_kmi(data) {
        if ddk != kmi {
            return Err(fail(
                ASSET,
                "module validation",
                "--module",
                format!("the module's DDK path names `{ddk}`, but the target KMI is `{kmi}`"),
                "use a module built for this KMI",
            ));
        }
    }
    if let Some(vermagic) = elf::vermagic(data, &elf) {
        if let Some(xy) = elf::kernel_xy(&vermagic) {
            let suffix = format!("-{xy}");
            if !kmi.ends_with(&suffix) {
                return Err(fail(
                    ASSET,
                    "module validation",
                    "--module",
                    format!(
                        "the module's vermagic targets kernel {xy}, but the target KMI is `{kmi}`"
                    ),
                    format!("use a module whose kernel version matches `{kmi}`"),
                ));
            }
        }
    }
    Ok(())
}

/// Refuse to install a module for a KMI that contradicts the image's own kernel.
fn ensure_kmi_matches_image(kmi: &str, info: &boot::ImageInfo) -> Res<()> {
    if let Some(image_kmi) = &info.kmi {
        if image_kmi != kmi {
            return Err(fail(
                KMI,
                "KMI resolution",
                kmi.to_string(),
                format!(
                    "the image kernel reports `{image_kmi}`, but the selected KMI is `{kmi}`"
                ),
                format!(
                    "use `{image_kmi}` (or omit the explicit KMI / --kmi-from) so the module matches the kernel"
                ),
            ));
        }
    } else if let Some(xy) = info.kernel_release.as_deref().and_then(elf::kernel_xy) {
        let suffix = format!("-{xy}");
        if !kmi.ends_with(&suffix) {
            return Err(fail(
                KMI,
                "KMI resolution",
                kmi.to_string(),
                format!("the image kernel is {xy}, but the selected KMI is `{kmi}`"),
                format!(
                    "choose a KMI ending in `{suffix}` (or omit the explicit KMI / --kmi-from)"
                ),
            ));
        }
    }
    Ok(())
}

/// Turn a kernel `major.minor` into the single KMI this APK provides for it.
///
/// A kernel version alone is deliberately ambiguous: `5.10` ships under both android12 and
/// android13, and `5.15` under both android13 and android14. When the kernel banner does not
/// name its branch, the platform release in the ramdisk's `build.prop` separates them.
/// `source` names where the version came from, so the report always says what the choice
/// rests on. `report` is false for `--kmi-from`, whose caller prints its own attribution.
fn resolve_kernel_version(
    v: &str,
    apk_kmi: &[String],
    android: Option<&str>,
    source: &str,
    report: bool,
) -> Res<String> {
    let suffix = format!("-{v}");
    let cands: Vec<&String> = apk_kmi.iter().filter(|k| k.ends_with(&suffix)).collect();
    let (chosen, how) = match cands.len() {
        1 => (cands[0].clone(), format!("kernel {v}, from {source}")),
        0 => {
            return Err(fail(
                KMI,
                "KMI resolution",
                v.to_string(),
                format!("no KMI in this APK targets kernel {v}"),
                format!("this APK provides: {}", apk_kmi.join(", ")),
            ))
        }
        _ => {
            if let Some(release) = android {
                let want = format!("android{release}-{v}");
                if cands.iter().any(|k| k.as_str() == want) {
                    (
                        want,
                        format!("kernel {v} + Android {release} from the ramdisk, from {source}"),
                    )
                } else {
                    return Err(kernel_version_ambiguous(v, &cands));
                }
            } else {
                return Err(kernel_version_ambiguous(v, &cands));
            }
        }
    };
    if report {
        ui::field("KMI", format!("{}  ({how})", style::bold_yellow(&chosen)));
    }
    Ok(chosen)
}

fn kernel_version_ambiguous(v: &str, cands: &[&String]) -> err::Fail {
    fail(
        KMI,
        "KMI resolution",
        v.to_string(),
        format!("kernel {v} matches more than one KMI in this APK"),
        format!(
            "specify one of: {}",
            cands
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

fn patch(args: &Args) -> Res<()> {
    let started = Instant::now();
    ui::set_verbose(args.verbose || std::env::var_os("KSUPATCH_VERBOSE").is_some());
    ui::header(
        env!("CARGO_PKG_VERSION"),
        "Offline KernelSU LKM boot-image patcher",
    );

    // ---- stage 1: inputs -----------------------------------------------------------
    stage(1, "Validate inputs");
    let apk_path = PathBuf::from(&args.positional[0]);
    let image_path = PathBuf::from(&args.positional[1]);
    for (p, what, hint) in [
        (
            &apk_path,
            "apk",
            "the FIRST argument is the KernelSU-family Manager .apk -- any file name is \
             fine; check the path, and quote it if it contains spaces",
        ),
        (
            &image_path,
            "image",
            "the SECOND argument is the boot.img, init_boot.img or vendor_boot.img -- \
             check the path, and quote it if it contains spaces",
        ),
    ] {
        let meta = std::fs::metadata(p).map_err(|e| {
            let mut hint = hint.to_string();
            if what == "image" {
                let found = nearby_images(p);
                if let Some(same) = found.iter().find(|c| c.file_name() == p.file_name()) {
                    hint = format!(
                        "{hint}; found `{}` -- did you mean that?",
                        display_path(same)
                    );
                } else if !found.is_empty() {
                    hint = format!(
                        "{hint}; images under this folder: {}",
                        found
                            .iter()
                            .take(4)
                            .map(|c| display_path(c))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
            fail(
                INPUT,
                "input validation",
                p.display().to_string(),
                format!("cannot stat {what}: {e}"),
                hint,
            )
        })?;
        if !meta.is_file() {
            return Err(fail(
                INPUT,
                "input validation",
                p.display().to_string(),
                format!("{what} path is not a regular file"),
                hint,
            ));
        }
    }
    let image_data = std::fs::read(&image_path).map_err(|e| {
        fail(
            INPUT,
            "input validation",
            image_path.display().to_string(),
            format!("cannot read image: {e}"),
            "check permissions and free memory",
        )
    })?;
    let image_label = image_path.display().to_string();
    let apk_size = std::fs::metadata(&apk_path)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    ui::field(
        "APK",
        format!(
            "{}  [{}]",
            style::cyan(&apk_path.display().to_string()),
            ui::size(apk_size)
        ),
    );
    ui::field(
        "Image",
        format!(
            "{}  [{}]",
            style::cyan(&image_path.display().to_string()),
            ui::size(image_data.len())
        ),
    );

    // ---- stage 2: APK --------------------------------------------------------------
    stage(2, "Read manager APK");
    let zip = zip::Zip::open(&apk_path).map_err(|e| {
        fail(
            APK,
            "APK container",
            apk_path.display().to_string(),
            e,
            "this expects an unmodified KernelSU-family Manager .apk",
        )
    })?;
    let manifest = zip
        .find("AndroidManifest.xml")
        .and_then(|e| zip.read(e).ok())
        .and_then(|b| axml::parse(&b).ok())
        .unwrap_or_default();
    let manager = manager_name(manifest.package.as_deref().unwrap_or(""));
    ui::field("Manager", style::bold_cyan(manager));
    ui::field(
        "Package",
        manifest.package.as_deref().unwrap_or("(unknown)"),
    );
    ui::field(
        "Version",
        format!(
            "{} ({})",
            manifest.version_name.as_deref().unwrap_or("(unknown)"),
            manifest
                .version_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_string())
        ),
    );
    let libs = find_libs(&zip, &apk_path)?;
    ui::field("ABIs", libs.len());

    // The KMI list is the same in every ABI's copy, so one read is enough. Prefer a
    // 64-bit ABI: a fork may also ship a 32-bit container that carries no `.ko` at all.
    // It is needed before the KMI stage so an explicit value can be validated and a bare
    // kernel version can be resolved.
    let kmi_lib = libs
        .iter()
        .find(|e| e.name.starts_with("lib/arm64-v8a/"))
        .or_else(|| libs.iter().find(|e| e.name.starts_with("lib/x86_64/")))
        .or_else(|| libs.first())
        .expect("find_libs never returns an empty vector");
    let apk_kmi = assets::supported_kmi(&zip.read(kmi_lib).map_err(|e| {
        fail(
            APK,
            "APK extraction",
            kmi_lib.name.clone(),
            e,
            "the APK may be truncated or corrupted; re-download it",
        )
    })?);
    ui::field("KMIs", style::bold_yellow(&apk_kmi.join(", ")));

    // ---- stage 3: image ------------------------------------------------------------
    stage(3, "Inspect image");
    let info = boot::inspect(&image_data, &image_label)?;
    ui::field(
        "Type",
        style::bold(&format!("{} (header v{})", info.kind, info.version)),
    );
    ui::field("Page size", info.page_size);
    ui::field("Image", ui::bytes(info.image_size));
    match &info.kernel {
        Some(k) => ui::field(
            "Kernel",
            format!(
                "{} stored -> {} plain ({})",
                ui::size(k.stored_len),
                k.plain_len.map(ui::size).unwrap_or_else(|| "?".into()),
                k.format
            ),
        ),
        None => ui::field(
            "Kernel",
            if info.kind == "vendor_boot" {
                "none (vendor_boot carries no kernel)"
            } else {
                "none (init_boot carries only the ramdisk)"
            },
        ),
    }
    if let Some(e) = &info.kernel_error {
        ui::warn(&format!(
            "the kernel section could not be decompressed: {e}"
        ));
        ui::note("the ramdisk can still be patched; the KMI must come from a module, a sibling boot.img or an explicit value");
    }
    match &info.ramdisk {
        Some(r) => ui::field(
            "Ramdisk",
            format!("{} ({})", ui::bytes(r.stored_len), r.format),
        ),
        None => ui::field("Ramdisk", "none"),
    }
    for (name, len) in &info.extra_blocks {
        ui::field(name, ui::bytes(*len));
    }
    if let Some(rel) = &info.kernel_release {
        ui::field("Kernel release", style::cyan(rel));
    }
    if let Some(a) = info.arch {
        ui::field("Arch", format!("{a} (from the kernel image)"));
    }
    if info.kind == "vendor_boot" {
        ui::field(
            "Fragments",
            info.ramdisk_fragments
                .iter()
                .map(|(name, size)| {
                    format!(
                        "{} {}",
                        if name.is_empty() { "(unnamed)" } else { name },
                        ui::bytes(*size)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        if let Some(name) = &info.ramdisk_name {
            ui::field(
                "Install into",
                format!(
                    "vendor ramdisk `{}`",
                    if name.is_empty() {
                        "(unnamed)"
                    } else {
                        name.as_str()
                    }
                ),
            );
        }
    }
    let (mut cpio, ramdisk_target) = boot::load_ramdisk_cpio(&image_data, &image_label)?;
    let already = cpio.exists(boot::MODULE_NAME) || cpio.exists("sevenk.ko");
    ui::field(
        "Ramdisk cpio",
        format!(
            "{} entries{}",
            cpio.entries().len(),
            if already { ", already patched" } else { "" }
        ),
    );
    if cpio.is_magisk_patched() {
        ui::warn("this ramdisk carries Magisk markers");
    }
    let platform = boot::android_release(&cpio);
    if let Some(rel) = &platform {
        ui::field(
            "Platform",
            format!("Android {rel} (from build.prop; does not decide the KMI)"),
        );
    }

    // ---- stage 4: KMI --------------------------------------------------------------
    stage(4, "Resolve KMI");
    let requested = args.positional.get(2).map(|s| s.trim().to_string());
    let from_image = match args.kmi_from.as_deref() {
        Some(path) => {
            let probe = std::fs::read(path).map_err(|e| {
                fail(
                    INPUT,
                    "input validation",
                    path.to_string(),
                    format!("cannot read --kmi-from: {e}"),
                    "point it at the boot.img from the same firmware",
                )
            })?;
            let (_, kmi, release) = boot::kernel_facts_from_image(&probe).ok_or_else(|| {
                fail(
                    KMI,
                    "KMI resolution",
                    path.to_string(),
                    "the image given to --kmi-from has no readable kernel section",
                    "point --kmi-from at a boot.img (the kernel lives there, not in \
                         init_boot.img)",
                )
            })?;
            let kmi = match kmi {
                Some(kmi) => kmi,
                None => {
                    let xy = release.as_deref().and_then(elf::kernel_xy).ok_or_else(|| {
                        fail(
                            KMI,
                            "KMI resolution",
                            path.to_string(),
                            "the --kmi-from image has a kernel but no readable release",
                            "pass a bare kernel version (e.g. `6.6`) instead, or use the \
                             firmware's boot.img",
                        )
                    })?;
                    resolve_kernel_version(
                        &xy,
                        &apk_kmi,
                        platform.as_deref(),
                        &format!("--kmi-from `{path}`"),
                        false,
                    )?
                }
            };
            Some(kmi)
        }
        None => None,
    };
    let kmi = match requested.as_deref() {
        Some(v) if !v.is_empty() && v != "auto" => {
            // Accept both the full KMI (`android15-6.6`) and a bare kernel version (`6.6`),
            // because that is what a user can actually read off a device.
            let chosen = if apk_kmi.iter().any(|k| k == v) {
                ui::field("KMI", format!("{} (explicit)", style::bold_yellow(v)));
                v.to_string()
            } else if let Some((xy, android)) = parse_kernel_input(v) {
                ui::field("Kernel", style::cyan(v));
                if let Some(android) = android {
                    // The release names its branch, so no build.prop guesswork is needed.
                    let want = format!("{android}-{xy}");
                    if apk_kmi.iter().any(|k| k == &want) {
                        ui::field(
                            "KMI",
                            format!(
                                "{}  (from the full kernel release; kernel {xy}, {android})",
                                style::bold_yellow(&want)
                            ),
                        );
                        want
                    } else {
                        return Err(fail(
                            KMI,
                            "KMI resolution",
                            v.to_string(),
                            format!(
                                "the kernel release names `{want}`, but this APK does not provide \
                                 that KMI"
                            ),
                            format!("this APK provides: {}", apk_kmi.join(", ")),
                        ));
                    }
                } else {
                    let suffix = format!("-{xy}");
                    if let Some(k) = info.kmi.as_deref().filter(|k| k.ends_with(&suffix)) {
                        ui::field(
                            "KMI",
                            format!(
                                "{}  (from the kernel banner; kernel {xy})",
                                style::bold_yellow(k)
                            ),
                        );
                        k.to_string()
                    } else {
                        resolve_kernel_version(&xy, &apk_kmi, platform.as_deref(), v, true)?
                    }
                }
            } else {
                return Err(fail(
                    KMI,
                    "KMI resolution",
                    v.to_string(),
                    format!("`{v}` is neither a KMI this APK provides nor a kernel version"),
                    format!(
                        "use one of: {} -- or a kernel version such as `6.6` or \
                         `6.6.77-android15-8-g1a2b3c4d5e6f`",
                        apk_kmi.join(", ")
                    ),
                ));
            };
            if let Some(from) = &from_image {
                if from != &chosen {
                    return Err(fail(
                        KMI,
                        "KMI resolution",
                        v.to_string(),
                        format!(
                            "`--kmi-from` reports `{from}`, but the explicit KMI resolves to `{chosen}`"
                        ),
                        "use the same KMI in both places, or omit one of them",
                    ));
                }
            }
            ensure_kmi_matches_image(&chosen, &info)?;
            chosen
        }
        _ => {
            // Automatic resolution, in order of how direct the evidence is. A KMI names a
            // GKI kernel branch plus that branch's kernel version, so what is needed is the
            // KERNEL's version -- never the Android platform version, which a ramdisk's
            // build.prop reports and which can point at the wrong module entirely.
            if let Some(k) = &from_image {
                ensure_kmi_matches_image(k, &info)?;
                ui::field(
                    "KMI",
                    format!("{}  (from --kmi-from)", style::bold_yellow(k)),
                );
                k.clone()
            } else if let Some(k) = &info.kmi {
                ui::field(
                    "KMI",
                    format!(
                        "{}  (auto-detected from the kernel inside the image)",
                        style::bold_yellow(k)
                    ),
                );
                k.clone()
            } else if let Some(xy) = info.kernel_release.as_deref().and_then(elf::kernel_xy) {
                // The kernel banner states the version but names no Android branch (a
                // custom kernel, or one whose localversion was changed). The ramdisk's
                // build.prop is enough to separate the two releases that share it.
                ui::field(
                    "Kernel",
                    format!(
                        "{xy}  (auto-detected from the kernel banner `{}`)",
                        info.kernel_release.as_deref().unwrap_or("")
                    ),
                );
                resolve_kernel_version(
                    &xy,
                    &apk_kmi,
                    platform.as_deref(),
                    "the kernel banner",
                    true,
                )?
            } else if let Some((xy, where_from)) = boot::kernel_xy_from_ramdisk(&cpio) {
                // An init_boot has no kernel, but a module already in its ramdisk states the
                // kernel it was built for. This is the same fact `uname -r` would give.
                ui::field(
                    "Kernel",
                    format!(
                        "{xy}  (auto-detected from the vermagic of `{where_from}` in the ramdisk)"
                    ),
                );
                resolve_kernel_version(
                    &xy,
                    &apk_kmi,
                    platform.as_deref(),
                    &format!("`{where_from}` in the ramdisk"),
                    true,
                )?
            } else if let Some((found, where_from)) =
                discover_sibling_kernel(&image_path, boot::arch_from_init(&cpio))
            {
                // The firmware ships boot.img next to init_boot.img; read the kernel from it
                // rather than making the user type a version.
                if is_kernel_version(&found) {
                    ui::field(
                        "Kernel",
                        format!("{found}  (auto-detected from the sibling `{where_from}`)"),
                    );
                    resolve_kernel_version(
                        &found,
                        &apk_kmi,
                        platform.as_deref(),
                        &format!("sibling `{where_from}`"),
                        true,
                    )?
                } else {
                    ui::field(
                        "KMI",
                        format!(
                            "{}  (auto-detected from the kernel in the sibling `{where_from}`)",
                            style::bold_yellow(&found)
                        ),
                    );
                    found
                }
            } else {
                let mut reason = String::from(
                    "no kernel version could be established offline. This image has no kernel \
                     section (init_boot only carries a ramdisk), and its ramdisk holds no \
                     kernel module whose vermagic would state the kernel it was built for. A \
                     KMI names a GKI kernel branch plus that branch's kernel version, so it \
                     follows the KERNEL, not the Android platform version that the ramdisk's \
                     build.prop reports",
                );
                if let Some(e) = &info.kernel_error {
                    reason.push_str(&format!(
                        " (the kernel section was present but could not be decompressed: {e})"
                    ));
                }
                return Err(fail(
                    KMI,
                    "KMI resolution",
                    image_label,
                    reason,
                    format!(
                        "put the firmware's boot.img (or a .ko from it) in the same folder \
                         as this image and it will be read automatically, or pass a bare \
                         kernel version (e.g. `6.6`), or use --kmi-from <image>. This APK \
                         provides: {}",
                        apk_kmi.join(", ")
                    ),
                ));
            }
        }
    };

    // ---- stage 5: payload ----------------------------------------------------------
    stage(5, "Select payload");
    let arch = match args.arch.as_deref() {
        Some(a) => {
            if abi_of(a).is_none() {
                return Err(fail(
                    USAGE,
                    "argument parsing",
                    a.to_string(),
                    format!("unsupported --arch `{a}`"),
                    "use aarch64, x86_64 or riscv64",
                ));
            }
            ui::field("Arch", format!("{a} (explicit)"));
            a.to_string()
        }
        None => match info.arch {
            Some(a) => a.to_string(),
            None => match boot::arch_from_init(&cpio) {
                Some(a) => {
                    ui::field("Arch", format!("{a} (from the ramdisk init)"));
                    a.to_string()
                }
                None => {
                    ui::field("Arch", "aarch64 (default; override with --arch)");
                    "aarch64".to_string()
                }
            },
        },
    };
    let Some(abi) = abi_of(&arch) else {
        return Err(fail(
            ASSET,
            "payload selection",
            arch.clone(),
            format!("this build cannot install into a `{arch}` image"),
            "supported architectures: aarch64, riscv64, x86_64",
        ));
    };
    let Some(target) = libs
        .iter()
        .position(|e| e.name.starts_with(&format!("lib/{abi}/")))
    else {
        return Err(fail(
            ASSET,
            "payload selection",
            apk_path.display().to_string(),
            format!("the APK has no native library for `lib/{abi}/`, required for arch {arch}"),
            format!(
                "available: {}",
                libs.iter()
                    .map(|e| e.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    };

    // Scan every native library, not just the one being patched: a KMI whose kernel version
    // is shared by two Android releases (5.10, 5.15) can only be told apart with a
    // toolchain anchor, and the other ABI is often the only place that anchor exists.
    let mut scans: Vec<Scanned> = Vec::new();
    for lib_entry in &libs {
        scans.push(scan_lib(&zip, lib_entry)?);
    }
    let mut seed: Vec<(String, String)> = Vec::new();
    for s in &scans {
        for pair in assets::ddk_anchors(&s.found) {
            if !seed.contains(&pair) {
                seed.push(pair);
            }
        }
    }
    ui::field(
        "Scanned",
        scans
            .iter()
            .map(|s| format!("{} ({} assets)", s.name, s.found.len()))
            .collect::<Vec<_>>()
            .join(", "),
    );
    if !seed.is_empty() {
        ui::field(
            "Anchors",
            seed.iter()
                .map(|(a, c)| format!("{a} <-> clang {c}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }

    let scan = &mut scans[target];
    ui::field("Library", style::cyan(&scan.name));
    ui::field("KMIs", style::bold_yellow(&scan.supported.join(", ")));
    if args.module.is_none() && !scan.supported.iter().any(|s| s == &kmi) {
        if scan.supported.is_empty() {
            return Err(fail(
                ASSET,
                "payload selection",
                format!("{arch} in {}", scan.name),
                format!("the `{arch}` payload in this APK carries no kernel module"),
                "this APK ships no `.ko` for that architecture (riscv64 currently has none); \
                 use an APK that does, or patch an arm64 image",
            ));
        }
        return Err(fail(
            KMI,
            "payload selection",
            kmi.clone(),
            format!("this APK does not provide a module for `{kmi}`"),
            format!("choose one of: {}", scan.supported.join(", ")),
        ));
    }

    let module: Vec<u8> = match args.module.as_deref() {
        Some(p) => {
            let d = std::fs::read(p).map_err(|e| {
                fail(
                    INPUT,
                    "input validation",
                    p.to_string(),
                    format!("cannot read --module: {e}"),
                    "check the path",
                )
            })?;
            validate_module(&d, &arch, &kmi)?;
            ui::field(
                "Module",
                format!("{p} ({}), supplied with --module", ui::size(d.len())),
            );
            d
        }
        None => {
            let judge = assets::judge(&mut scan.found, &scan.supported, &seed);
            let module_idx = judge
                .assigned
                .iter()
                .find(|(_, k)| k == &kmi)
                .map(|(i, _)| *i)
                .ok_or_else(|| {
                    fail(
                        ASSET,
                        "payload selection",
                        format!("{kmi} in {}", scan.name),
                        format!(
                            "the `{kmi}` module could not be identified inside the APK ({} modules unresolved)",
                            judge.unresolved.len()
                        ),
                        "pass --module <path to a module for this KMI> to supply it explicitly",
                    )
                })?;
            let a = &scan.found[module_idx];
            ui::field(
                "Module",
                format!(
                    "{} (KMI {}, {})",
                    style::bold(&ui::size(a.data.len())),
                    style::bold_yellow(a.kmi.as_deref().unwrap_or(&kmi)),
                    a.arch()
                ),
            );
            for line in &a.evidence {
                ui::field("Evidence", line);
            }
            a.data.clone()
        }
    };
    let ksuinit = scan
        .found
        .iter()
        .find(|a| a.kind == assets::Kind::KsuInit)
        .ok_or_else(|| {
            fail(
                ASSET,
                "payload selection",
                scan.name.clone(),
                "no ksuinit asset found in the APK",
                "this APK may be for a KernelSU build without LKM support",
            )
        })?;
    ui::field("KsuInit", ui::bytes(ksuinit.data.len()));

    // Where the result goes.
    let (out_path, _out_name) = output_paths(args)?;

    // ---- stage 6: injection --------------------------------------------------------
    stage(6, "Inject into ramdisk");
    let ksu_config: Option<String> = args.ksu_config.clone();
    let stock_sha1 = if args.no_stock_backup {
        None
    } else {
        Some(sha1::hex(&image_data))
    };
    let outcome = boot::inject(
        &mut cpio,
        &boot::InjectOptions {
            module: &module,
            module_name: scan.module_file,
            ksuinit: &ksuinit.data,
            ksu_config: ksu_config.as_deref(),
            stock_sha1: stock_sha1.as_deref(),
        },
        &image_label,
    )?;
    ui::field(
        "Mode",
        if outcome.upgrade {
            "refresh of an existing KernelSU install"
        } else {
            "fresh install"
        },
    );
    ui::field(
        "Init",
        format!(
            "{} ({}), mode {:o}{}",
            boot::INIT_NAME,
            ui::size(outcome.init_len),
            boot::INIT_MODE,
            if outcome.renamed_init {
                format!("; original moved to {}", boot::INIT_REAL_NAME)
            } else {
                String::new()
            }
        ),
    );
    ui::field(
        "Module",
        format!(
            "{} ({})",
            style::bold(scan.module_file),
            ui::size(outcome.module_len)
        ),
    );
    if let Some(cfg) = &outcome.config {
        ui::field("Config", format!("{cfg} in {}", boot::CONFIG_NAME));
    } else {
        ui::field(
            "Config",
            format!("none ({} not written)", boot::CONFIG_NAME),
        );
    }
    if let Some(sha) = &outcome.stock_sha1 {
        ui::field("Stock backup", format!("{} = {sha}", boot::STOCK_SHA1_NAME));
    }
    if outcome.missing_init {
        ui::warn(&format!(
            "the ramdisk had no `init` entry, so there was nothing to chain to -- {} may not start",
            boot::INIT_NAME
        ));
    }

    // ---- stage 7: repack -----------------------------------------------------------
    stage(7, "Repack image");
    let out_bytes = boot::repack(&image_data, &cpio, &ramdisk_target, &image_label)?;

    let out_path = write_new_unique(&out_path, &out_bytes)?;
    ui::field(
        "Image",
        format!(
            "{} -> {}",
            ui::bytes(image_data.len()),
            ui::bytes(out_bytes.len())
        ),
    );
    ui::field(
        "Written",
        style::bold_green(&out_path.display().to_string()),
    );

    // ---- stage 8: verification -----------------------------------------------------
    if args.verify {
        stage(8, "Verify output");
        let v = boot::verify(
            &out_bytes,
            &out_path.display().to_string(),
            &boot::VerifyOptions {
                module_name: scan.module_file,
                module: &module,
                init: &ksuinit.data,
                require_init_real: true,
            },
        )?;
        ui::field(
            "Header",
            format!("v{}, {}", v.version, ui::bytes(v.image_size)),
        );
        ui::field(
            "Module",
            format!(
                "{} present ({})",
                style::bold(scan.module_file),
                ui::size(v.module_len)
            ),
        );
        ui::field(
            "Init",
            format!(
                "{} present ({})",
                style::bold(boot::INIT_NAME),
                ui::size(v.init_len)
            ),
        );
        ui::field("Init.real", if v.init_real { "present" } else { "ABSENT" });
        ui::field("Config", v.config.as_deref().unwrap_or("none"));
        ui::field("Result", style::bold_green("PASS"));
    } else {
        stage(8, "Verify output");
        ui::note("skipped (pass --verify to re-check the output)");
    }

    if ui::verbose() {
        ui::done(
            "PATCH COMPLETE",
            &[
                (
                    "Result",
                    if args.verify {
                        "verified".to_string()
                    } else {
                        "patched (not verified)".to_string()
                    },
                ),
                ("Manager", manager.to_string()),
                ("KMI", kmi.clone()),
                ("Arch", arch.clone()),
                ("Module", ui::bytes(outcome.module_len)),
                (
                    "Output",
                    format!("{}  [{}]", out_path.display(), ui::size(out_bytes.len())),
                ),
                ("Time", format!("{:.2}s", started.elapsed().as_secs_f64())),
            ],
        );
    } else {
        let result = if args.verify {
            style::bold_green("verified")
        } else {
            "patched (unverified)".to_string()
        };
        ui::summary(&[
            (
                "APK",
                format!(
                    "{}  [{}]",
                    style::cyan(&apk_path.display().to_string()),
                    ui::size(apk_size)
                ),
            ),
            (
                "Image",
                format!(
                    "{}  [{}]",
                    style::cyan(&image_path.display().to_string()),
                    ui::size(image_data.len())
                ),
            ),
            ("", String::new()),
            (
                "Manager",
                format!(
                    "{}  {}",
                    style::bold_cyan(manager),
                    manifest.version_name.as_deref().unwrap_or("")
                ),
            ),
            ("KMI", style::bold_yellow(&kmi)),
            (
                "Module",
                format!(
                    "{}  {}",
                    style::bold(scan.module_file),
                    ui::size(outcome.module_len)
                ),
            ),
            (
                "Output",
                format!(
                    "{}  [{}]",
                    style::bold_green(&out_path.display().to_string()),
                    ui::size(out_bytes.len())
                ),
            ),
            (
                "Result",
                format!("{result}  {:.2}s", started.elapsed().as_secs_f64()),
            ),
        ]);
    };
    Ok(())
}

// =====================================================================================
// Output path
// =====================================================================================

/// Write bytes to a new file, adding `-1`, `-2`, ... when the name already exists.
///
/// `create_new` makes the choice atomic, so two concurrent runs cannot both take the same
/// timestamped name.
fn write_new_unique(path: &Path, data: &[u8]) -> Res<PathBuf> {
    let mut candidate = path.to_path_buf();
    for attempt in 0..1000usize {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(data) {
                    // Remove the zero-length or partially written file before returning;
                    // otherwise a full disk leaves a truncated image behind and the next
                    // run silently picks a "-1" name instead of failing cleanly.
                    drop(file);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(fail(
                        IO,
                        "output",
                        candidate.display().to_string(),
                        format!("cannot write output image: {e}"),
                        "check free space and permissions",
                    ));
                }
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate = numbered_path(path, attempt + 1);
            }
            Err(e) => {
                return Err(fail(
                    IO,
                    "output",
                    candidate.display().to_string(),
                    format!("cannot create output image: {e}"),
                    "check free space and permissions",
                ))
            }
        }
    }
    Err(fail(
        IO,
        "output",
        path.display().to_string(),
        "could not find a free output file name",
        "remove an old output image or choose another directory",
    ))
}

fn numbered_path(path: &Path, n: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let name = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => format!("{stem}-{n}.{ext}"),
        None => format!("{stem}-{n}"),
    };
    path.with_file_name(name)
}

/// Resolve `--output` / `--out` / `--out-name` into a concrete file path.
fn output_paths(args: &Args) -> Res<(PathBuf, String)> {
    let mut out_dir = PathBuf::from(".");
    let mut out_name: Option<String> = None;
    if let Some(p) = args.output.as_deref() {
        let path = PathBuf::from(p);
        let is_dir = p.ends_with('/') || p.ends_with('\\') || path.is_dir();
        if is_dir {
            out_dir = path;
        } else {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    out_dir = parent.to_path_buf();
                }
            }
            out_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string);
        }
    }
    if let Some(d) = args.out.as_deref() {
        out_dir = PathBuf::from(d);
    }
    if let Some(n) = args.out_name.as_deref() {
        let mut parts = Path::new(n).components();
        let valid =
            matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none();
        if !valid {
            return Err(fail(
                USAGE,
                "argument parsing",
                n.to_string(),
                "`--out-name` must be a plain file name",
                "use `--out-name kernel.img`, or pass a full path with `--output`",
            ));
        }
        out_name = Some(n.to_string());
    }
    let out_name = out_name.unwrap_or_else(|| format!("kernelsu_patched_{}.img", timestamp()));
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        fail(
            IO,
            "output",
            out_dir.display().to_string(),
            format!("cannot create output directory: {e}"),
            "check permissions",
        )
    })?;
    Ok((out_dir.join(&out_name), out_name))
}

// =====================================================================================
// helpers
// =====================================================================================

/// The embedded asset blob is emitted as a `static` inside the native library, so it lives
/// in `.rodata` (with `.data.rel.ro` as a possible home). Bounding the brute-force search
/// to those sections removes the long futile tail after the last asset. The sections are
/// scanned separately rather than as one span: `.data.rel.ro` sits far above `.rodata`, so
/// a union would drag in `.text` and every frame table in between.
fn scan_ranges(lib: &[u8]) -> Vec<(usize, usize)> {
    let Some(info) = elf::ElfInfo::parse(lib) else {
        return vec![(0, lib.len())];
    };
    let mut out: Vec<(usize, usize)> = Vec::new();
    for name in [".rodata", ".data.rel.ro"] {
        if let Some((off, size)) = info.section(lib, name) {
            if size > 0 {
                out.push((off, off + size));
            }
        }
    }
    if out.is_empty() {
        out.push((0, lib.len()));
    }
    out
}

/// Start a numbered step.
fn stage(n: u8, what: &str) {
    ui::step(n, 8, what);
}

fn group(n: usize) -> String {
    ui::group(n)
}

/// Path relative to the current directory, for hints.
fn display_path(path: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    path.strip_prefix(&cwd)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn collect_images(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("img"))
        {
            out.push(path);
        }
    }
}

/// Images in the working directory, one level below it, and next to the requested path.
///
/// Used only to make a missing-image error actionable: `output\init_boot.img` may simply be
/// `temp\init_boot.img` in the same folder.
fn nearby_images(wanted: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let cwd = std::env::current_dir().unwrap_or_default();
    collect_images(&cwd, &mut out);
    if let Ok(entries) = std::fs::read_dir(&cwd) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() && !matches!(name.as_ref(), "target" | ".git" | "node_modules") {
                collect_images(&path, &mut out);
            }
        }
    }
    if let Some(parent) = wanted.parent() {
        collect_images(parent, &mut out);
    }
    out.sort();
    out.dedup();
    out.truncate(8);
    out
}

/// UTC timestamp, so output files never collide.
fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::parse_kernel_input;

    #[test]
    fn parses_bare_and_full_kernel_releases() {
        assert_eq!(parse_kernel_input("5.10"), Some(("5.10".into(), None)));
        assert_eq!(parse_kernel_input("5.10.198"), Some(("5.10".into(), None)));
        assert_eq!(
            parse_kernel_input("5.10.198.1"),
            Some(("5.10".into(), None))
        );
        assert_eq!(
            parse_kernel_input("5.10.198-android13"),
            Some(("5.10".into(), Some("android13".into())))
        );
        assert_eq!(
            parse_kernel_input("5.10.198-android13-8-g1a2b3c4d5e6f"),
            Some(("5.10".into(), Some("android13".into())))
        );
        assert_eq!(
            parse_kernel_input("6.6.77-android15-8-gdeadbeef"),
            Some(("6.6".into(), Some("android15".into())))
        );
        assert_eq!(
            parse_kernel_input("5.10.198-g1a2b3c4d5e6f"),
            Some(("5.10".into(), None))
        );
    }

    #[test]
    fn rejects_things_that_are_not_kernel_releases() {
        for bad in [
            "",
            "5",
            "5.x",
            "5.10foo",
            "5.10.198foo",
            "android13-5.10",
            "5.10-android",
            "5.10-android13x",
        ] {
            assert_eq!(parse_kernel_input(bad), None, "should reject {bad:?}");
        }
    }
}
