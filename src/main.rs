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
mod zip;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use err::{fail, report, Res, APK, ASSET, INPUT, IO, KMI, USAGE};

const RULE: usize = 78;

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
ksupatch {} -- offline KernelSU LKM boot-image patcher

USAGE
  ksupatch <kernelsu.apk> <boot.img|init_boot.img> [<kmi>|auto] [options]
  ksupatch --dump-assets <kernelsu.apk>

ARGUMENTS
  kernelsu.apk    Official KernelSU Manager APK. It supplies the kernel module and
                  ksuinit, and anchors the result to that exact release.
  boot.img        Stock GKI boot or init_boot image (header v3/v4) to patch.
  kmi             Target KMI, e.g. android14-6.1, or a bare kernel version such as 6.6.
                  Use `auto` (or omit) to have it worked out automatically.

  A KMI names a GKI kernel branch plus that branch's kernel version (android15-6.6 = the
  android15 branch's 6.6 kernel), so it follows the KERNEL, never the Android platform
  version that a ramdisk's build.prop reports.

  `auto` uses the first of these that applies:
    1. the kernel inside the image,             -- for boot.img
    2. the vermagic of a kernel module already in the ramdisk,
                                                -- for an init_boot whose ramdisk holds
                                                   one (e.g. from an earlier install)
    3. the firmware's `boot.img`, if it sits next to this image
                                                -- it carries the kernel; the exact name
                                                   `boot.img` is required
    4. --kmi-from <image>
  An init_boot with no kernel, no module in its ramdisk and no sibling to read has to be
  told: init_boot is the partition Android 13 carved out to hold only the ramdisk, so the
  kernel version is simply not in that file.

OPTIONS
  -o, --output <p>  Where to write the result. Give a file path, or a directory (with a
                    trailing separator) to use the default name inside it. Optional: the
                    default is the current directory with a timestamped name.
  --out <dir>       Directory for the produced image (default: current directory).
  --out-name <f>    File name for the produced image (default: timestamped).
  --module <path>   Use this kernelsu.ko instead of the one inside the APK.
  --arch <arch>     aarch64 (default) or x86_64. Normally detected from the image.
  --kmi-from <img>  Read the kernel version from this image instead (a boot.img).
  --color <when>    auto (default), always or never. `auto` colours only when stdout is a
                    terminal, so redirected logs stay free of escape sequences; NO_COLOR
                    is honoured too.
  --verify          Re-parse the produced image and assert the install is present.
  --dump-assets     List the assets inside the APK and exit.
  --ksu-config <t>  Write a `ksu_config` file with these tokens. By default none is written,
                    which is what an install does when the module is supplied as a file.
  --no-stock-backup Skip recording `stock_image.sha1`. By default it is recorded with the
                    SHA-1 of the input image, which is what a backed-up install does.
  --help            Show this text.

The defaults reproduce what a KernelSU install on a device produces, byte for byte.

EXIT CODES
  0 ok   1 usage   2 input   3 apk   4 image   5 kmi   6 asset   7 inject   8 io

EXAMPLE
  ksupatch KernelSU.apk init_boot.img android14-6.1 --verify
  ksupatch KernelSU.apk boot.img auto --out out\\",
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
                    "expected: ksupatch <kernelsu.apk> <boot.img|init_boot.img> [<kmi>|auto] \
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
    if argv.is_empty() || argv[0] == "--help" || argv[0] == "-h" {
        println!("{}", usage());
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
                "ksupatch --dump-assets <kernelsu.apk>",
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
            "ksupatch <kernelsu.apk> <boot.img|init_boot.img> [<kmi>|auto]",
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
    extract_secs: f64,
    scan_secs: f64,
}

fn dump_assets(apk: &Path) -> Res<()> {
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
            "pass the KernelSU .apk file itself",
        ));
    }

    println!("KernelSU APK asset inspection");
    rule();
    println!("  apk            : {}", apk.display());
    println!("  file size      : {} bytes", group(meta.len() as usize));

    let zip = zip::Zip::open(&apk).map_err(|e| {
        fail(
            APK,
            "APK container",
            apk.display().to_string(),
            e,
            "this expects an unmodified KernelSU release .apk (a ZIP container)",
        )
    })?;
    println!("  zip entries    : {}", zip.entries.len());
    print_manifest(&zip);

    let libs = find_libs(&zip, &apk)?;
    println!(
        "  native libs    : {}",
        libs.iter()
            .map(|e| e.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();

    println!("pass 1 -- extract and scan");
    let mut scans: Vec<Scanned> = Vec::new();
    for entry in &libs {
        let s = scan_lib(&zip, entry)?;
        println!(
            "  {:<26} {} -> {} bytes, {} assets  (extract {:.2}s, scan {:.2}s)",
            s.name,
            group(s.csize as usize),
            group(s.lib_len),
            s.found.len(),
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
        println!("  pooled KMI anchors learned from DDK build paths:");
        for (android, clang) in &seed {
            println!("    {android} <-> clang {clang}");
        }
    }

    println!();
    for s in scans.iter_mut() {
        report_lib(s, &seed);
    }

    rule();
    println!("finished in {:.2}s", started.elapsed().as_secs_f64());
    Ok(())
}

fn print_manifest(zip: &zip::Zip) {
    match zip.find("AndroidManifest.xml") {
        Some(e) => match zip.read(e).and_then(|b| axml::parse(&b)) {
            Ok(m) => {
                println!(
                    "  package        : {}",
                    m.package.as_deref().unwrap_or("(not found)")
                );
                println!(
                    "  version        : {}",
                    m.version_name.as_deref().unwrap_or("(not found)")
                );
                println!(
                    "  version code   : {}",
                    m.version_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "(not found)".to_string())
                );
            }
            Err(e) => println!("  manifest       : could not be read ({e})"),
        },
        None => println!("  manifest       : AndroidManifest.xml is absent"),
    }
}

fn find_libs<'a>(zip: &'a zip::Zip, apk: &Path) -> Res<Vec<zip::Entry>> {
    let libs: Vec<zip::Entry> = zip
        .entries
        .iter()
        .filter(|e| e.name.starts_with("lib/") && e.name.ends_with("/libksud.so"))
        .cloned()
        .collect();
    if libs.is_empty() {
        return Err(fail(
            APK,
            "APK contents",
            apk.display().to_string(),
            "no lib/<abi>/libksud.so entry found, so this APK carries no KernelSU payload",
            "make sure this is the KernelSU Manager APK (it contains libksud.so under lib/)",
        ));
    }
    Ok(libs)
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
        found,
        extract_secs,
        scan_secs,
    })
}

fn report_lib(s: &mut Scanned, seed: &[(String, String)]) {
    rule();
    println!("{}", s.name);
    rule();
    println!("  extracted      : {} bytes", group(s.lib_len));
    let total: usize = s.ranges.iter().map(|(a, b)| b - a).sum();
    println!(
        "  scanned        : {} bytes in {} range(s): {}",
        group(total),
        s.ranges.len(),
        s.ranges
            .iter()
            .map(|(a, b)| format!("{a:#X}..{b:#X}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  assets found   : {}   (raw-DEFLATE streams that inflate to an ELF object)",
        s.found.len()
    );
    println!();
    println!("  KMI names this APK declares ({}):", s.supported.len());
    for k in &s.supported {
        println!("    - {k}");
    }
    println!();
    println!(
        "  {:<3} {:<10} {:<10} {:<10} {:<8} {:<8} {}",
        "#", "offset", "csize", "usize", "arch", "type", "identity"
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

    let judge = assets::judge(&mut s.found, &s.supported, seed);
    println!(
        "  module -> KMI resolution ({} of {})",
        judge.assigned.len(),
        judge.module_count
    );
    let mut by_kmi: Vec<&(usize, String)> = judge.assigned.iter().collect();
    by_kmi.sort_by(|a, b| a.1.cmp(&b.1));
    for (idx, kmi) in by_kmi {
        let a = &s.found[*idx];
        println!(
            "    {:<16} usize {:<10} vermagic {}",
            kmi,
            group(a.data.len()),
            a.vermagic.as_deref().unwrap_or("(absent)")
        );
        for line in &a.evidence {
            println!("        {}", style::dim(&format!("evidence: {line}")));
        }
    }
    if !judge.unresolved.is_empty() {
        println!();
        println!("    UNRESOLVED -- KMI could not be determined uniquely:");
        for idx in &judge.unresolved {
            let a = &s.found[*idx];
            println!(
                "      usize {:<10} vermagic {:<58} ddk {}  clang {}",
                group(a.data.len()),
                a.vermagic.as_deref().unwrap_or("-"),
                a.ddk_kmi.as_deref().unwrap_or("-"),
                a.clang.as_deref().unwrap_or("-")
            );
        }
    }
    self_check(&s.found, &judge, &s.supported);
    println!();
}

fn self_check(found: &[assets::Asset], judge: &assets::Judge, supported: &[String]) {
    println!();
    println!("  self-check");
    let mut failed = 0usize;
    let mut check = |pass: bool, what: String| {
        println!(
            "    [{}] {}",
            if pass { style::green("ok") } else { style::bold_red("FAIL") },
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
    let ksuinit = found.iter().filter(|a| a.kind == assets::Kind::KsuInit).count();
    check(
        ksuinit == 1,
        format!("exactly one ksuinit recovered (found {ksuinit})"),
    );
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
    let (arch, kmi) = boot::kernel_facts_from_image(&data)?;
    if expect_arch.map(|want| want == arch).unwrap_or(true) {
        if let Some(kmi) = kmi {
            return Some((kmi, name));
        }
    }
    None
}

/// Turn a kernel `major.minor` into the single KMI this APK provides for it.
///
/// `source` names where the version came from, so the report always says what the choice
/// rests on.
fn resolve_kernel_version(v: &str, apk_kmi: &[String], source: &str) -> Res<String> {
    let suffix = format!("-{v}");
    let cands: Vec<&String> = apk_kmi.iter().filter(|k| k.ends_with(&suffix)).collect();
    match cands.len() {
        1 => {
            println!(
                "      resolved      : {} (kernel {v} identifies it, from {source})",
                cands[0]
            );
            Ok(cands[0].clone())
        }
        0 => Err(fail(
            KMI,
            "KMI resolution",
            v.to_string(),
            format!("no KMI in this APK targets kernel {v}"),
            format!("this APK provides: {}", apk_kmi.join(", ")),
        )),
        _ => Err(fail(
            KMI,
            "KMI resolution",
            v.to_string(),
            format!("kernel {v} matches more than one KMI in this APK"),
            format!(
                "specify one of: {}",
                cands.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ),
        )),
    }
}

fn patch(args: &Args) -> Res<()> {
    let started = Instant::now();
    println!(
        "{}",
        style::bold(&format!(
            "=== ksupatch v{} === KernelSU LKM boot image patch",
            env!("CARGO_PKG_VERSION")
        ))
    );
    rule();

    // ---- stage 1: inputs -----------------------------------------------------------
    stage(1, "validate inputs");
    let apk_path = PathBuf::from(&args.positional[0]);
    let image_path = PathBuf::from(&args.positional[1]);
    for (p, what, hint) in [
        (
            &apk_path,
            "apk",
            "the FIRST argument is the KernelSU .apk -- check the path, and quote it if it \
             contains spaces",
        ),
        (
            &image_path,
            "image",
            "the SECOND argument is the boot.img or init_boot.img -- check the path, and \
             quote it if it contains spaces",
        ),
    ] {
        let meta = std::fs::metadata(p).map_err(|e| {
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
        println!("      {:<6} {}  ({} bytes)", what, p.display(), group(meta.len() as usize));
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

    // ---- stage 2: APK --------------------------------------------------------------
    stage(2, "read KernelSU APK");
    let zip = zip::Zip::open(&apk_path).map_err(|e| {
        fail(
            APK,
            "APK container",
            apk_path.display().to_string(),
            e,
            "this expects an unmodified KernelSU release .apk",
        )
    })?;
    let manifest = zip
        .find("AndroidManifest.xml")
        .and_then(|e| zip.read(e).ok())
        .and_then(|b| axml::parse(&b).ok())
        .unwrap_or_default();
    println!(
        "      package {}  version {}  versionCode {}",
        manifest.package.as_deref().unwrap_or("(unknown)"),
        manifest.version_name.as_deref().unwrap_or("(unknown)"),
        manifest
            .version_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "(unknown)".to_string())
    );
    let libs = find_libs(&zip, &apk_path)?;
    println!("      native libs: {}", libs.len());

    // The KMI list is the same in every ABI's copy, so one read is enough. It is needed
    // before the KMI stage so an explicit value can be validated and a bare kernel version
    // can be resolved.
    let apk_kmi = assets::supported_kmi(&zip.read(&libs[0]).map_err(|e| {
        fail(
            APK,
            "APK extraction",
            libs[0].name.clone(),
            e,
            "the APK may be truncated or corrupted; re-download it",
        )
    })?);
    println!("      KMI offered : {}", apk_kmi.join(", "));

    // ---- stage 3: image ------------------------------------------------------------
    stage(3, "inspect image");
    let info = boot::inspect(&image_data, &image_label)?;
    println!("      type         : {} (header v{})", info.kind, info.version);
    println!("      page size    : {}", info.page_size);
    println!("      image size   : {} bytes", group(info.image_size));
    match &info.kernel {
        Some(k) => println!(
            "      kernel       : {} bytes stored, {} bytes plain, format {}",
            group(k.stored_len),
            k.plain_len.map(group).unwrap_or_else(|| "?".into()),
            k.format
        ),
        None => println!("      kernel       : none (this is an init_boot image)"),
    }
    match &info.ramdisk {
        Some(r) => println!(
            "      ramdisk      : {} bytes stored, format {}",
            group(r.stored_len),
            r.format
        ),
        None => println!("      ramdisk      : none"),
    }
    for (name, len) in &info.extra_blocks {
        println!("      block        : {} ({} bytes)", name, group(*len));
    }
    if let Some(rel) = &info.kernel_release {
        println!("      kernel release: {rel}");
    }
    if let Some(a) = info.arch {
        println!("      arch          : {a} (from kernel image)");
    }

    let mut cpio = boot::load_ramdisk_cpio(&image_data, &image_label)?;
    let already = cpio.exists(boot::MODULE_NAME);
    println!(
        "      ramdisk cpio  : {} entries{}",
        cpio.entries().len(),
        if already { ", already KernelSU-patched" } else { "" }
    );
    if cpio.is_magisk_patched() {
println!(
            "      {} this ramdisk carries Magisk markers",
            style::yellow("WARNING       :")
        );
    }
    if let Some(rel) = boot::android_release(&cpio) {
        println!(
            "      platform      : Android {rel} (from the ramdisk's build.prop; this does \
             NOT decide the KMI -- the kernel does)"
        );
    }

    // ---- stage 4: KMI --------------------------------------------------------------
    stage(4, "resolve KMI");
    let requested = args.positional.get(2).map(|s| s.trim().to_string());
    let kmi = match requested.as_deref() {
        Some(v) if !v.is_empty() && v != "auto" => {
            // Accept both the full KMI (`android15-6.6`) and a bare kernel version (`6.6`),
            // because that is what a user can actually read off a device.
            if apk_kmi.iter().any(|k| k == v) {
                println!("      requested     : {v} (explicit)");
                v.to_string()
            } else if is_kernel_version(v) {
                println!("      requested     : {v} (kernel version)");
                resolve_kernel_version(v, &apk_kmi, v)?
            } else {
                return Err(fail(
                    KMI,
                    "KMI resolution",
                    v.to_string(),
                    format!("`{v}` is neither a KMI this APK provides nor a kernel version"),
                    format!(
                        "use one of: {} -- or a bare kernel version such as `6.6`",
                        apk_kmi.join(", ")
                    ),
                ));
            }
        }
        _ => {
            // Automatic resolution, in order of how direct the evidence is. A KMI names a
            // GKI kernel branch plus that branch's kernel version, so what is needed is the
            // KERNEL's version -- never the Android platform version, which a ramdisk's
            // build.prop reports and which can point at the wrong module entirely.
            if let Some(path) = args.kmi_from.as_deref() {
                let probe = std::fs::read(path).map_err(|e| {
                    fail(
                        INPUT,
                        "input validation",
                        path.to_string(),
                        format!("cannot read --kmi-from: {e}"),
                        "point it at the boot.img from the same firmware",
                    )
                })?;
                match boot::kmi_from_image(&probe) {
                    Some(k) => {
                        println!("      auto-detected : {k} (from the kernel in {path})");
                        k
                    }
                    None => {
                        return Err(fail(
                            KMI,
                            "KMI resolution",
                            path.to_string(),
                            "the image given to --kmi-from has no kernel section either",
                            "point --kmi-from at a boot.img (the kernel lives there, not in \
                             init_boot.img)",
                        ))
                    }
                }
            } else if let Some(k) = &info.kmi {
                println!("      auto-detected : {k} (from the kernel inside the image)");
                k.clone()
            } else if let Some((xy, where_from)) = boot::kernel_xy_from_ramdisk(&cpio) {
                // An init_boot has no kernel, but a module already in its ramdisk states the
                // kernel it was built for. This is the same fact `uname -r` would give.
                println!(
                    "      auto-detected : kernel {xy} (from the vermagic of `{where_from}` \
                     in the ramdisk)"
                );
                resolve_kernel_version(&xy, &apk_kmi, &format!("`{where_from}` in the ramdisk"))?
            } else if let Some((found, where_from)) =
                discover_sibling_kernel(&image_path, boot::arch_from_init(&cpio))
            {
                // The firmware ships boot.img next to init_boot.img; read the kernel from it
                // rather than making the user type a version.
                if is_kernel_version(&found) {
                    println!(
                        "      auto-detected : kernel {found} (from the sibling `{where_from}`)"
                    );
                    resolve_kernel_version(&found, &apk_kmi, &format!("sibling `{where_from}`"))?
                } else {
                    println!(
                        "      auto-detected : {found} (from the kernel in the sibling `{where_from}`)"
                    );
                    found
                }
            } else {
                return Err(fail(
                    KMI,
                    "KMI resolution",
                    image_label,
                    "no kernel version could be established offline. This image has no kernel \
                     section (init_boot only carries a ramdisk), and its ramdisk holds no \
                     kernel module whose vermagic would state the kernel it was built for. A \
                     KMI names a GKI kernel branch plus that branch's kernel version, so it \
                     follows the KERNEL, not the Android platform version that the ramdisk's \
                     build.prop reports",
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
    stage(5, "select payload");
    let arch = match args.arch.as_deref() {
        Some(a) => {
            if abi_of(a).is_none() {
                return Err(fail(
                    USAGE,
                    "argument parsing",
                    a.to_string(),
                    format!("unsupported --arch `{a}`"),
                    "use aarch64 or x86_64",
                ));
            }
            println!("      arch          : {a} (explicit)");
            a.to_string()
        }
        None => match info.arch {
            Some(a) => a.to_string(),
            None => match boot::arch_from_init(&cpio) {
                Some(a) => {
                    println!("      arch          : {a} (from the ramdisk's init binary)");
                    a.to_string()
                }
                None => {
                    println!(
                        "      arch          : aarch64 (default; the image gives no hint -- \
                         override with --arch)"
                    );
                    "aarch64".to_string()
                }
            },
        },
    };
    let abi = abi_of(&arch).unwrap();
    let Some(target) = libs.iter().position(|e| e.name.starts_with(&format!("lib/{abi}/"))) else {
        return Err(fail(
            ASSET,
            "payload selection",
            apk_path.display().to_string(),
            format!("the APK has no `lib/{abi}/libksud.so`, required for arch {arch}"),
            format!(
                "available: {}",
                libs.iter().map(|e| e.name.clone()).collect::<Vec<_>>().join(", ")
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
    println!(
        "      scanned libs  : {}",
        scans
            .iter()
            .map(|s| format!("{} ({} assets)", s.name, s.found.len()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if !seed.is_empty() {
        println!(
            "      pooled anchors: {}",
            seed.iter()
                .map(|(a, c)| format!("{a}<->clang {c}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let scan = &mut scans[target];
    println!("      lib           : {}", scan.name);
    println!("      KMI supported : {}", scan.supported.join(", "));
    if !scan.supported.iter().any(|s| s == &kmi) {
        return Err(fail(
            KMI,
            "payload selection",
            kmi.clone(),
            format!("this APK does not provide a module for `{kmi}`"),
            format!("choose one of: {}", scan.supported.join(", ")),
        ));
    }

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
                "pass --module <path to kernelsu.ko> to supply it explicitly",
            )
        })?;
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
            println!("      module        : {p} ({} bytes, from --module)", group(d.len()));
            d
        }
        None => {
            let a = &scan.found[module_idx];
            println!(
                "      module        : {} bytes from APK (from {} in a {})",
                group(a.data.len()),
                a.kmi.as_deref().unwrap_or(&kmi),
                a.arch()
            );
            for line in &a.evidence {
                println!("      {} {line}", style::dim("evidence      :"));
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
    println!(
        "      ksuinit       : {} bytes from APK",
        group(ksuinit.data.len())
    );

    // ---- stage 6: injection --------------------------------------------------------
    stage(6, "inject into ramdisk");
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
            ksuinit: &ksuinit.data,
            ksu_config: ksu_config.as_deref(),
            stock_sha1: stock_sha1.as_deref(),
        },
        &image_label,
    )?;
    println!(
        "      mode          : {}",
        if outcome.upgrade {
            "refresh of an existing KernelSU install"
        } else {
            "fresh install"
        }
    );
    println!(
        "      init          : {} ({} bytes, mode {:o}) {}",
        boot::INIT_NAME,
        group(outcome.init_len),
        boot::INIT_MODE,
        if outcome.renamed_init {
            format!("installed; original moved to {}", boot::INIT_REAL_NAME)
        } else {
            "installed".to_string()
        }
    );
    println!(
        "      module        : {} ({} bytes)",
        boot::MODULE_NAME,
        group(outcome.module_len)
    );
    if let Some(cfg) = &outcome.config {
        println!("      config        : {cfg} in {}", boot::CONFIG_NAME);
    } else {
        println!(
            "      config        : none ({} not written)",
            boot::CONFIG_NAME
        );
    }
    if let Some(sha) = &outcome.stock_sha1 {
        println!("      stock backup  : {} = {sha}", boot::STOCK_SHA1_NAME);
    }
    if outcome.missing_init {
        println!(
            "      {} the ramdisk had no `init` entry, so there was nothing to chain to -- \
             {} may not start",
            style::yellow("WARNING       :"),
            boot::INIT_NAME
        );
    }

    // ---- stage 7: repack -----------------------------------------------------------
    stage(7, "repack image");
    let out_bytes = boot::repack(&image_data, &cpio, &image_label)?;

    // Where the result goes. `--output` takes a file path, like `-o` elsewhere; if it names
    // a directory (trailing separator, or an existing directory) the default file name is
    // used inside it. `--out` and `--out-name` can still pin each part separately.
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
    let out_path = out_dir.join(&out_name);
    std::fs::write(&out_path, &out_bytes).map_err(|e| {
        fail(
            IO,
            "output",
            out_path.display().to_string(),
            format!("cannot write output image: {e}"),
            "check free space and permissions",
        )
    })?;
    println!("      image         : {} bytes -> {} bytes", group(image_data.len()), group(out_bytes.len()));
    println!(
        "      written       : {}",
        style::bold_green(&out_path.display().to_string())
    );

    // ---- stage 8: verification -----------------------------------------------------
    if args.verify {
        stage(8, "verify output");
        let v = boot::verify(&out_bytes, &out_path.display().to_string())?;
        println!(
            "      re-parsed     : header v{}, {} bytes",
            v.version,
            group(v.image_size)
        );
        println!("      module        : {} present, {} bytes", boot::MODULE_NAME, group(v.module_len));
        println!("      init          : {} present, {} bytes", boot::INIT_NAME, group(v.init_len));
        println!(
            "      init.real     : {}",
            if v.init_real { "present" } else { "ABSENT" }
        );
        println!(
            "      config        : {}",
            v.config.as_deref().unwrap_or("(none)")
        );
        println!("      result        : {}", style::bold_green("PASS"));
    } else {
        stage(8, "verification skipped (pass --verify to re-check the output)");
    }

    rule();
    println!(
        "=== done in {:.2}s -- flash {} to the partition you extracted it from",
        started.elapsed().as_secs_f64(),
        out_name
    );
    Ok(())
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

/// One stage heading, e.g. `[3/8] inspect image`.
fn stage(n: u8, what: &str) {
    println!("{}", style::heading(&format!("[{n}/8] {what}")));
}

/// A dimmed horizontal rule.
fn rule() {
    println!("{}", style::dim(&"=".repeat(RULE)));
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
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
