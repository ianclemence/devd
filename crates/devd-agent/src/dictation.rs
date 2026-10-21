//! Speech to text, on this machine.
//!
//! The window has a microphone button, because typing a long instruction is
//! tiresome and talking is quick. What happens behind that button is the same
//! thing Laravel Herd does, and the same thing [Handy](https://github.com/cjpais/Handy)
//! does: the audio is recorded locally, silenced parts are dropped, and a local
//! Whisper model transcribes it. Nothing is sent to a cloud service, because
//! the words in a development prompt are usually about the code on your disk.
//!
//! # The pieces
//!
//! * A **binary** - `whisper-cli` from [whisper.cpp](https://github.com/ggml-org/whisper.cpp),
//!   which publishes a signed zip for Windows and is built from source
//!   elsewhere. devd installs the published build and says plainly when there
//!   is none for the platform.
//! * A **model** - the GGML Whisper models, which are published on Hugging Face
//!   with a digest for each file.
//! * An **audio file** - 16 kHz mono PCM WAV, which is what Whisper wants. The
//!   browser records it that way, so nothing has to decode a compressed
//!   format here.
//!
//! The digests are the upstream ones. A model that does not match its digest
//! is refused rather than used.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use devd_core::capability::Ctx;
use devd_core::download::Artifact;
use devd_core::{DevdError, Result};

/// The whisper.cpp release devd installs the binary from.
pub const WHISPER_RELEASE: &str = "v1.7.6";

/// Where the Windows x64 build of `whisper-cli` is published.
///
/// The release asset carries a digest, so the download is checked against
/// something upstream states.
pub const WHISPER_BINARY_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/v1.7.6/whisper-bin-x64.zip";

/// The digest of that asset.
pub const WHISPER_BINARY_SHA256: &str =
    "0d2eca299c248f965bd0341bcb219db4b433c7f0c0ce2200d4df85765e8156a9";

/// The size of that asset, in bytes.
pub const WHISPER_BINARY_BYTES: u64 = 3_675_974;

/// A Whisper model devd can install.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Model {
    /// Identifier used on the command line.
    pub id: &'static str,
    /// What the model is, in words.
    pub label: &'static str,
    /// The file name upstream publishes it under.
    pub filename: &'static str,
    /// Where it is published.
    pub url: &'static str,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256, as upstream states it.
    pub sha256: &'static str,
}

/// The models devd offers, smallest first.
///
/// The English-only models are smaller and better at English, which is what a
/// development prompt usually is. The multilingual ones are there for the rest.
pub const MODELS: &[Model] = &[
    Model {
        id: "tiny.en",
        label: "Tiny (English)",
        filename: "ggml-tiny.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin",
        size: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
    },
    Model {
        id: "base.en",
        label: "Base (English)",
        filename: "ggml-base.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
        size: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
    },
    Model {
        id: "small.en",
        label: "Small (English)",
        filename: "ggml-small.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
        size: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
    },
];

/// A model by its identifier.
pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

/// Where the Whisper files live.
pub fn dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("bin").join("whisper")
}

/// Where the models live.
pub fn models_dir(ctx: &Ctx) -> PathBuf {
    dir(ctx).join("models")
}

/// Where the binary lives once it is installed.
pub fn binary_path(ctx: &Ctx) -> PathBuf {
    dir(ctx).join(if cfg!(windows) { "whisper-cli.exe" } else { "whisper-cli" })
}

/// Where a model lives once it is installed.
pub fn model_path(ctx: &Ctx, id: &str) -> PathBuf {
    models_dir(ctx).join(format!("ggml-{id}.bin"))
}

/// Whether this platform has a published `whisper-cli` build.
///
/// Windows does. Elsewhere the binary is built from source, and devd says so
/// rather than downloading something it cannot vouch for.
pub fn platform_supported() -> bool {
    cfg!(windows)
}

/// What is installed, and what could be installed.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// Whether a `whisper-cli` binary is present.
    pub binary: bool,
    /// Where the binary is, or would be.
    pub binary_path: String,
    /// Whether this platform has a published build.
    pub platform_supported: bool,
    /// The models that are installed.
    pub models: Vec<InstalledModel>,
    /// The models that could be installed.
    pub available: Vec<Model>,
}

/// One installed model.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledModel {
    /// Its identifier.
    pub id: String,
    /// Where it is.
    pub path: String,
    /// Its size in bytes.
    pub bytes: u64,
}

/// The current state of the dictation tools.
pub fn status(ctx: &Ctx) -> Status {
    let mut models = Vec::new();
    for m in MODELS {
        let path = model_path(ctx, m.id);
        if let Ok(meta) = std::fs::metadata(&path) {
            models.push(InstalledModel {
                id: m.id.to_string(),
                path: path.display().to_string(),
                bytes: meta.len(),
            });
        }
    }
    Status {
        binary: binary_path(ctx).is_file(),
        binary_path: binary_path(ctx).display().to_string(),
        platform_supported: platform_supported(),
        models,
        available: MODELS.to_vec(),
    }
}

/// Install the binary and a model.
///
/// Both are checked against the digest upstream publishes, so a download that
/// does not match is refused rather than used.
pub fn install(ctx: &Ctx, model_id: &str) -> Result<Status> {
    if !platform_supported() {
        return Err(DevdError::unsupported(
            "there is no published whisper-cli build for this platform; build \
             whisper.cpp from source and point devd at the binary",
        ));
    }
    let model = model(model_id).ok_or_else(|| {
        DevdError::invalid(format!(
            "unknown model `{model_id}`; the models are {}",
            MODELS
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;

    // The binary.
    let binary = binary_path(ctx);
    if !binary.is_file() {
        let artifact = Artifact {
            url: WHISPER_BINARY_URL.to_string(),
            filename: "whisper-bin-x64.zip".to_string(),
            sha256: Some(WHISPER_BINARY_SHA256.to_string()),
            bytes: Some(WHISPER_BINARY_BYTES),
        };
        let fetched = devd_core::download::fetch(&artifact, &ctx.paths.cache(), None)?;
        std::fs::create_dir_all(dir(ctx))?;
        devd_core::archive::extract(&fetched.path, &dir(ctx), true)?;
        if !binary.is_file() {
            return Err(DevdError::internal(format!(
                "the archive did not contain {}",
                binary.display()
            )));
        }
        make_executable(&binary)?;
    }

    // The model.
    let target = model_path(ctx, model.id);
    if !target.is_file() {
        let artifact = Artifact {
            url: model.url.to_string(),
            filename: model.filename.to_string(),
            sha256: Some(model.sha256.to_string()),
            bytes: Some(model.size),
        };
        let fetched = devd_core::download::fetch(&artifact, &ctx.paths.cache(), None)?;
        std::fs::create_dir_all(models_dir(ctx))?;
        std::fs::copy(&fetched.path, &target)?;
    }

    Ok(status(ctx))
}

/// Make a file runnable, where the platform has such a thing.
fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Transcribe a WAV file.
///
/// The file must be 16 kHz mono PCM, which is what the recorder produces.
/// The text comes back with the trailing blank lines whisper.cpp adds removed.
pub fn transcribe(ctx: &Ctx, wav: &Path, language: Option<&str>) -> Result<String> {
    let binary = binary_path(ctx);
    if !binary.is_file() {
        return Err(DevdError::not_found(
            "Whisper is not installed; run `devd dictation install base.en` to install it",
        ));
    }
    if !wav.is_file() {
        return Err(DevdError::not_found(format!(
            "{} does not exist",
            wav.display()
        )));
    }
    let model = MODELS
        .iter()
        .find(|m| model_path(ctx, m.id).is_file())
        .ok_or_else(|| {
            DevdError::not_found(
                "no Whisper model is installed; run `devd dictation install base.en` to install one",
            )
        })?;

    // whisper-cli writes the transcript next to the file it was given, so the
    // output is written into a directory of its own and read back.
    let work = ctx.paths.root().join("tmp").join("dictation");
    std::fs::create_dir_all(&work)?;
    let stem = work.join("transcript");
    let out = stem.with_extension("txt");
    let _ = std::fs::remove_file(&out);

    let mut command = std::process::Command::new(&binary);
    command
        .arg("-m")
        .arg(model_path(ctx, model.id))
        .arg("-f")
        .arg(wav)
        .arg("-nt")
        .arg("-otxt")
        .arg("-of")
        .arg(&stem)
        .arg("-np");
    if let Some(language) = language.filter(|l| !l.trim().is_empty()) {
        command.arg("-l").arg(language.trim());
    }
    command
        .current_dir(&work)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let result = command.output().map_err(|e| {
        DevdError::Command {
            program: binary.display().to_string(),
            code: -1,
            stderr: format!("whisper-cli could not be started: {e}"),
        }
    })?;
    if !result.status.success() {
        return Err(DevdError::Command {
            program: binary.display().to_string(),
            code: result.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&result.stderr)
                .lines()
                .filter(|l| l.contains("error") || l.contains("failed"))
                .take(4)
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }

    let text = std::fs::read_to_string(&out).map_err(|e| {
        DevdError::internal(format!(
            "whisper-cli finished but wrote no transcript: {e}"
        ))
    })?;
    Ok(text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" "))
}

/// The state of the dictation tools, as JSON.
pub fn status_json(ctx: &Ctx) -> serde_json::Value {
    json!(status(ctx))
}
