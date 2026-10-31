//! `derive-creds`: gets the account's CLOB API credentials (key, secret, passphrase), which
//! PolyBolt needs, and writes them to an env file outside the repo.
//!
//! The signer's private key is read from a hidden prompt (or `POLY_PRIVATE_KEY`), used for one
//! L1 signature and never written anywhere. Idempotent: an account that already has credentials
//! gets the same ones back. It signs no orders and moves no funds.

use std::fs::{DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use anyhow::{Context, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::Parser;
use polymarket_client_sdk_v2::POLYGON;
use polymarket_client_sdk_v2::auth::{ExposeSecret as _, LocalSigner, Signer as _};
use polymarket_client_sdk_v2::clob::{Client, Config};
use secrecy::SecretString;

#[derive(Parser)]
#[command(about = "Derives the CLOB API credentials and writes them to an env file outside the repo")]
struct Args {
    /// Where to write the env file [default: ~/.config/raki-trader/poly.env]. Must not be inside a
    /// git checkout.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Replace an existing file.
    #[arg(long)]
    force: bool,
    /// CLOB V2 host.
    #[arg(long, default_value = "https://clob.polymarket.com")]
    host: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // aws-lc-rs and ring are both linked; without an explicit default rustls panics.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let args = Args::parse();
    let out = match args.out {
        Some(p) => p,
        None => default_out()?,
    };
    // Before the key is asked for, so a bad path costs nothing.
    check_out_path(&out, args.force)?;

    let key = private_key()?;
    let signer = LocalSigner::from_str(unquote(key.expose_secret()))
        .map_err(|_| anyhow::anyhow!("the private key is not a valid 32-byte hex key"))?
        .with_chain_id(Some(POLYGON));
    drop(key);
    let address = signer.address().to_string();
    println!("signer address: {address}");
    match std::env::var("POLY_ADDRESS") {
        Ok(expected) if expected.trim().eq_ignore_ascii_case(&address) => {
            println!("it matches POLY_ADDRESS");
        }
        Ok(expected) if !expected.trim().is_empty() => println!(
            "POLY_ADDRESS is {}, a different address. That is expected if it is your deposit \
             wallet (the address polymarket.com shows for deposits): the credentials belong to \
             the signer. Keep both; orders later need the deposit wallet as the funder.",
            expected.trim()
        ),
        _ => {}
    }

    let client = Client::new(&args.host, Config::default())?;
    let creds = client
        .create_or_derive_api_key(&signer, None)
        .await
        .context("derive the CLOB API credentials")?;
    let api_key = creds.key().to_string();
    let body = env_file(
        &address,
        &api_key,
        creds.secret().expose_secret(),
        creds.passphrase().expose_secret(),
        Utc::now(),
    )?;
    write_private(&out, &body, args.force)?;
    println!(
        "wrote the credentials for API key {}… to {} (mode 600)",
        &api_key[..api_key.len().min(8)],
        out.display()
    );
    Ok(())
}

fn default_out() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set; pass --out")?;
    Ok(PathBuf::from(home).join(".config/raki-trader/poly.env"))
}

fn private_key() -> anyhow::Result<SecretString> {
    if let Ok(k) = std::env::var("POLY_PRIVATE_KEY")
        && !k.trim().is_empty()
    {
        println!("using POLY_PRIVATE_KEY from the environment");
        return Ok(SecretString::from(k));
    }
    let k = rpassword::prompt_password("signer private key (input hidden): ")
        .context("read the private key")?;
    if k.trim().is_empty() {
        bail!("no private key entered");
    }
    Ok(SecretString::from(k))
}

/// The key as pasted from an env file: surrounding whitespace and one pair of quotes dropped.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    ['"', '\'']
        .iter()
        .find_map(|&q| s.strip_prefix(q)?.strip_suffix(q))
        .unwrap_or(s)
        .trim()
}

/// Refuses a path inside a git checkout (secrets never live in a repo) and an existing file
/// unless `force`.
fn check_out_path(out: &Path, force: bool) -> anyhow::Result<()> {
    let abs = std::path::absolute(out).with_context(|| format!("resolve {}", out.display()))?;
    if let Some(repo) = abs.ancestors().skip(1).find(|d| d.join(".git").exists()) {
        bail!(
            "{} is inside the git checkout {}; write it outside any repo",
            abs.display(),
            repo.display()
        );
    }
    if abs.exists() && !force {
        bail!("{} already exists; pass --force to replace it", abs.display());
    }
    Ok(())
}

/// The env file, sourceable by `sh` (`set -a; . file; set +a`).
fn env_file(
    address: &str,
    api_key: &str,
    secret: &str,
    passphrase: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    for (name, v) in [
        ("api key", api_key),
        ("secret", secret),
        ("passphrase", passphrase),
    ] {
        if v.is_empty() || v.contains(['\'', '\n', '\r']) {
            bail!("the {name} from the CLOB is empty or not safe to quote");
        }
    }
    Ok(format!(
        "# CLOB API credentials for signer {address}, derived {} by derive-creds.\n\
         # Secret: never commit, paste or share. Paper mode needs only these, not the private key.\n\
         POLY_API_KEY='{api_key}'\n\
         POLY_API_SECRET='{secret}'\n\
         POLY_API_PASSPHRASE='{passphrase}'\n",
        now.to_rfc3339_opts(SecondsFormat::Secs, true)
    ))
}

/// Writes `body` readable by the owner only, creating the parent directory (mode 700).
fn write_private(out: &Path, body: &str, force: bool) -> anyhow::Result<()> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("create {}", dir.display()))?;
    }
    let mut opts = OpenOptions::new();
    opts.write(true).mode(0o600);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    let mut f = opts
        .open(out)
        .with_context(|| format!("open {}", out.display()))?;
    // `mode` applies only on creation; a replaced file may have been wider.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(body.as_bytes())?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_file_quotes_values_and_rejects_unsafe_ones() {
        let now = DateTime::from_timestamp(1_793_031_000, 0).unwrap();
        let body = env_file("0xabc", "k-1", "c2Vj+/=", "p4ss", now).unwrap();
        assert!(body.contains("derived 2026-10-26T16:10:00Z"));
        assert!(body.contains("POLY_API_KEY='k-1'\n"));
        assert!(body.contains("POLY_API_SECRET='c2Vj+/='\n"));
        assert!(body.contains("POLY_API_PASSPHRASE='p4ss'\n"));
        assert!(env_file("0xabc", "k", "it's", "p", now).is_err());
        assert!(env_file("0xabc", "k", "s", "a\nb", now).is_err());
        assert!(env_file("0xabc", "", "s", "p", now).is_err());
    }

    #[test]
    fn unquote_drops_one_pair_of_quotes() {
        assert_eq!(unquote("  0xab12\n"), "0xab12");
        assert_eq!(unquote("\"0xab12\""), "0xab12");
        assert_eq!(unquote("'0xab12'"), "0xab12");
        assert_eq!(unquote("\"0xab12'"), "\"0xab12'");
    }

    #[test]
    fn out_path_must_be_outside_a_git_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("frontend")).unwrap();
        assert!(check_out_path(&repo.join("frontend/.env"), false).is_err());
        assert!(check_out_path(&repo.join("poly.env"), true).is_err());
        assert!(check_out_path(&tmp.path().join("cfg/poly.env"), false).is_ok());
    }

    #[test]
    fn writes_owner_only_and_needs_force_to_replace() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("cfg/poly.env");
        write_private(&out, "A='1'\n", false).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&out), 0o600);
        assert_eq!(mode(out.parent().unwrap()), 0o700);
        assert!(check_out_path(&out, false).is_err());
        assert!(write_private(&out, "A='2'\n", false).is_err());

        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&out, "A='2'\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "A='2'\n");
        assert_eq!(mode(&out), 0o600);
    }
}
