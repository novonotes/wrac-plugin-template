use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Parser, Subcommand};

const PLUGIN_NAME: &str = "WRAC Gain";
const CLAP_BUNDLE_NAME: &str = "WRAC Gain.clap";
const VST3_BUNDLE_NAME: &str = "WRAC Gain.vst3";
const AU_BUNDLE_NAME: &str = "WRAC Gain.component";
const AU_TYPE: &str = "aufx";
const AU_SUBTYPE: &str = "WtGn";
const AU_MANUFACTURER: &str = "YrCo";

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Build and validate WRAC plugin artifacts")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// CLAP と wrapper format を build する。
    Build(BuildArgs),
    /// build 済み artifact を user-local plugin folder へ install する。
    Install(ProfileArgs),
    /// VST3 validator と、macOS では auval を実行する。
    Validate(ProfileArgs),
}

#[derive(Debug, Args)]
struct BuildArgs {
    /// release profile で build する。
    #[arg(long)]
    release: bool,

    /// build 前に生成済み plugin 出力を削除する。
    #[arg(long)]
    clean: bool,

    /// CLAP bundle だけを build する。
    #[arg(long)]
    clap_only: bool,

    /// CLAP build を skip し、既存 CLAP bundle から wrapper format だけを rebuild する。
    #[arg(long)]
    wrapper_only: bool,

    /// build 後に生成 artifact を install する。
    #[arg(long)]
    install: bool,

    /// build 後に生成された VST3/AU artifact を validate する。
    #[arg(long)]
    validate: bool,
}

#[derive(Debug, Args)]
struct ProfileArgs {
    /// release build の artifact を使う。
    #[arg(long)]
    release: bool,
}

#[derive(Debug, Clone, Copy)]
struct Profile {
    cmake: &'static str,
    script: &'static str,
}

impl Profile {
    fn from_release(release: bool) -> Self {
        if release {
            Self {
                cmake: "Release",
                script: "Release",
            }
        } else {
            Self {
                cmake: "Debug",
                script: "Debug",
            }
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = Context::new()?;

    match cli.command {
        Commands::Build(args) => build(&ctx, args)?,
        Commands::Install(args) => install(&ctx, Profile::from_release(args.release))?,
        Commands::Validate(args) => validate(&ctx, Profile::from_release(args.release))?,
    }

    Ok(())
}

struct Context {
    root: PathBuf,
    target_dir: PathBuf,
    wrapper_dir: PathBuf,
}

impl Context {
    fn new() -> Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(1)
            .ok_or("failed to locate repository root")?
            .to_path_buf();
        let target_dir = env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target"));
        let wrapper_dir = env::var_os("CLAP_WRAPPER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("clap_wrapper_builder"));

        Ok(Self {
            root,
            target_dir,
            wrapper_dir,
        })
    }

    fn script(&self, name: &str) -> PathBuf {
        self.root.join("script").join(name)
    }

    fn clap_bundle(&self) -> PathBuf {
        self.target_dir.join("bundled").join(CLAP_BUNDLE_NAME)
    }

    fn wrapper_build_dir(&self) -> PathBuf {
        self.wrapper_dir.join("build_WXP_Example_Gain")
    }

    fn vst3_bundle(&self, profile: Profile) -> PathBuf {
        self.wrapper_build_dir()
            .join(profile.cmake)
            .join(VST3_BUNDLE_NAME)
    }

    fn au_bundle(&self, profile: Profile) -> PathBuf {
        self.wrapper_build_dir()
            .join(profile.cmake)
            .join(AU_BUNDLE_NAME)
    }
}

fn build(ctx: &Context, args: BuildArgs) -> Result<()> {
    if args.clap_only && args.wrapper_only {
        return Err("--clap-only and --wrapper-only cannot be used together".into());
    }

    let profile = Profile::from_release(args.release);

    if args.clean {
        remove_if_exists(&ctx.clap_bundle())?;
        remove_if_exists(&ctx.wrapper_build_dir())?;
    }

    if !args.wrapper_only {
        run(Command::new(ctx.script("build.sh"))
            .arg(profile.script)
            .current_dir(&ctx.root))?;
    }

    if !args.clap_only {
        if !ctx.clap_bundle().exists() {
            return Err(format!("CLAP bundle not found: {}", ctx.clap_bundle().display()).into());
        }

        run(Command::new(ctx.script("build_wrapper.sh"))
            .arg(profile.script)
            .env("SKIP_CLAP_BUILD", "1")
            .current_dir(&ctx.root))?;
    }

    if args.install {
        install(ctx, profile)?;
    }

    if args.validate {
        validate(ctx, profile)?;
    }

    print_outputs(ctx, profile, args.clap_only);
    Ok(())
}

fn install(ctx: &Context, profile: Profile) -> Result<()> {
    run(Command::new(ctx.script("install.sh")).current_dir(&ctx.root))?;

    if cfg!(target_os = "linux") {
        return Ok(());
    }

    run(
        Command::new(ctx.wrapper_dir.join("install_wrapper_plugin.sh"))
            .arg(CLAP_BUNDLE_NAME)
            .arg(PLUGIN_NAME)
            .arg(profile.script)
            .current_dir(&ctx.wrapper_dir),
    )?;

    Ok(())
}

fn validate(ctx: &Context, profile: Profile) -> Result<()> {
    let vst3 = ctx.vst3_bundle(profile);
    if !vst3.exists() {
        return Err(format!("VST3 bundle not found: {}", vst3.display()).into());
    }

    let validator = ensure_vst3_validator(ctx)?;
    run(Command::new(validator).arg(&vst3).current_dir(&ctx.root))?;

    if cfg!(target_os = "macos") {
        let au = ctx.au_bundle(profile);
        if !au.exists() {
            return Err(format!("AU bundle not found: {}", au.display()).into());
        }

        let install_dir = home_dir()?.join("Library/Audio/Plug-Ins/Components");
        fs::create_dir_all(&install_dir)?;
        let installed = install_dir.join(AU_BUNDLE_NAME);
        remove_if_exists(&installed)?;
        // auval は任意 path の component ではなく AudioComponent registry を検証する。
        // user-local install 先へ `/bin/cp -R` で bundle として置き直し、DAW と同じ発見
        // 経路を通す。
        run(Command::new("/bin/cp").arg("-R").arg(&au).arg(&install_dir))?;

        let _ = Command::new("killall")
            .args(["-9", "AudioComponentRegistrar"])
            .status();

        run(Command::new("/usr/bin/auval")
            .args(["-v", AU_TYPE, AU_SUBTYPE, AU_MANUFACTURER])
            .current_dir(&ctx.root))?;
    }

    Ok(())
}

fn ensure_vst3_validator(ctx: &Context) -> Result<PathBuf> {
    let executable = if cfg!(target_os = "windows") {
        "validator.exe"
    } else {
        "validator"
    };
    let validator = ctx
        .target_dir
        .join("vst3sdk-validator")
        .join("bin")
        .join("Debug")
        .join(executable);

    if validator.exists() {
        return Ok(validator);
    }

    // validator は配布 artifact ではなく検証用 tool なので、plugin の profile に関係なく
    // Debug で用意する。Release plugin の validate でも同じ executable を使える。
    let build_dir = ctx.target_dir.join("vst3sdk-validator");
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(ctx.wrapper_dir.join("vst3sdk"))
        .arg("-B")
        .arg(&build_dir)
        .arg("-DSMTG_ENABLE_VST3_HOSTING_EXAMPLES=ON")
        .arg("-DSMTG_ENABLE_VST3_PLUGIN_EXAMPLES=OFF")
        .arg("-DSMTG_ENABLE_VSTGUI_SUPPORT=OFF");
    if cfg!(target_os = "macos") {
        configure.arg("-GXcode");
    }
    run(configure.current_dir(&ctx.root))?;

    run(Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("validator")
        .arg("--config")
        .arg("Debug")
        .current_dir(&ctx.root))?;

    if !validator.exists() {
        return Err(format!("validator executable not found: {}", validator.display()).into());
    }

    Ok(validator)
}

fn print_outputs(ctx: &Context, profile: Profile, clap_only: bool) {
    println!("CLAP: {}", ctx.clap_bundle().display());
    if !clap_only {
        println!("VST3: {}", ctx.vst3_bundle(profile).display());
        if cfg!(target_os = "macos") {
            println!("AU: {}", ctx.au_bundle(profile).display());
        }
    }
}

fn run(command: &mut Command) -> Result<()> {
    println!("$ {}", format_command(command));
    let status = command.status()?;
    if !status.success() {
        return Err(format!(
            "command failed with status {status}: {}",
            format_command(command)
        )
        .into());
    }
    Ok(())
}

fn format_command(command: &Command) -> String {
    let mut parts = Vec::new();
    parts.push(shell_display(command.get_program()));
    parts.extend(command.get_args().map(shell_display));
    parts.join(" ")
}

fn shell_display(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.into_owned()
    }
}

fn remove_if_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}
