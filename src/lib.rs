//! Read-only PhotoKit probe for LightCraft issue #1622.
//!
//! Answers two questions with facts, not guesses:
//! 1. Does reading Apple Photos through `objc2-photos` need `unsafe`? Every PhotoKit call below is
//!    wrapped in its own `unsafe` block and the crate warns on unneeded ones (`unused_unsafe`), so
//!    the build log lists the calls Rust treats as safe; the rest need `unsafe`.
//! 2. Who gets access? The same probe runs as a plain command-line tool, as one with an embedded
//!    Info.plist, and inside an .app bundle; each reports what macOS answered.
//!
//! It never changes the library: no `performChanges`, no writes. It prints album names, counts
//! and, for the first few photos, their local identifier, type, date, favourite flag and original
//! file names. Run it against a test library.
#![warn(unused_unsafe)]

pub mod edit;

use std::fmt::Write as _;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2_photos::{
    PHAccessLevel, PHAsset, PHAssetCollection, PHAssetCollectionSubtype, PHAssetCollectionType, PHAssetMediaType, PHAssetResource,
    PHAuthorizationStatus, PHPhotoLibrary,
};

/// What macOS says about this process's access to the Photos library.
pub fn status_name(s: PHAuthorizationStatus) -> String {
    let name = match s {
        PHAuthorizationStatus::NotDetermined => "notDetermined (never asked)",
        PHAuthorizationStatus::Restricted => "restricted (blocked by policy)",
        PHAuthorizationStatus::Denied => "denied",
        PHAuthorizationStatus::Authorized => "authorized (full access)",
        PHAuthorizationStatus::Limited => "limited (selected photos only)",
        _ => "unknown",
    };
    format!("{name} [{}]", s.0)
}

/// Access status without asking.
pub fn status() -> PHAuthorizationStatus {
    unsafe { PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite) }
}

/// Ask for access (macOS shows its prompt the first time) and wait for the answer, at most `wait`.
pub fn request(wait: Duration) -> Option<PHAuthorizationStatus> {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |s: PHAuthorizationStatus| {
        let _ = tx.send(s);
    });
    unsafe { PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(PHAccessLevel::ReadWrite, &handler) };
    rx.recv_timeout(wait).ok()
}

fn media_type(t: PHAssetMediaType) -> &'static str {
    match t {
        PHAssetMediaType::Image => "image",
        PHAssetMediaType::Video => "video",
        PHAssetMediaType::Audio => "audio",
        _ => "unknown",
    }
}

/// Albums (user albums at any level) with their photo counts, then the first `n` photos of the
/// library: identifier, type, date, favourite, size and their resources' original file names.
pub fn list(n: usize) -> String {
    let mut out = String::new();
    let albums = unsafe { PHAssetCollection::fetchAssetCollectionsWithType_subtype_options(PHAssetCollectionType::Album, PHAssetCollectionSubtype::Any, None) };
    let count = unsafe { albums.count() };
    let _ = writeln!(out, "albums: {count}");
    for i in 0..count {
        let album = unsafe { albums.objectAtIndex(i) };
        let title = unsafe { album.localizedTitle() }.map(|t| t.to_string()).unwrap_or_default();
        let inside = unsafe { PHAsset::fetchAssetsInAssetCollection_options(&album, None) };
        let photos = unsafe { inside.count() };
        let _ = writeln!(out, "  - {title:?}: {photos} item(s)");
    }
    let all = unsafe { PHAsset::fetchAssetsWithOptions(None) };
    let total = unsafe { all.count() };
    let _ = writeln!(out, "items in library: {total}");
    for i in 0..total.min(n) {
        let asset = unsafe { all.objectAtIndex(i) };
        let id = unsafe { asset.localIdentifier() }.to_string();
        let kind = media_type(unsafe { asset.mediaType() });
        let favorite = unsafe { asset.isFavorite() };
        let (w, h) = unsafe { (asset.pixelWidth(), asset.pixelHeight()) };
        let date = unsafe { asset.creationDate() }.map(|d| d.description().to_string()).unwrap_or_else(|| "-".into());
        let _ = writeln!(out, "  [{i}] {id} {kind} {w}x{h} favourite={favorite} created={date}");
        let resources = unsafe { PHAssetResource::assetResourcesForAsset(&asset) };
        for r in resources.iter() {
            let name = unsafe { r.originalFilename() }.to_string();
            let uti = unsafe { r.uniformTypeIdentifier() }.to_string();
            let kind = unsafe { r.r#type() }.0;
            let _ = writeln!(out, "      resource type {kind}: {name} ({uti})");
        }
    }
    out
}

/// Run the probe as `variant` with these arguments (`status`, `request`, or `list [N]`, the
/// default), print the report and copy it to /tmp/photokit-probe-<variant>.txt (an .app opened
/// from the Finder has no terminal to print to).
pub fn run(variant: &str, args: &[String]) {
    let mut report = String::new();
    let _ = writeln!(report, "photokit-probe {} ({variant})", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(report, "process: {}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());
    let before = status();
    let _ = writeln!(report, "status before: {}", status_name(before));
    let cmd = args.first().map(String::as_str).unwrap_or("list");
    match cmd {
        "status" => {}
        "request" | "list" => {
            let mut now = before;
            if now == PHAuthorizationStatus::NotDetermined {
                let _ = writeln!(report, "asking macOS for access (a prompt may appear)...");
                match request(Duration::from_secs(120)) {
                    Some(s) => now = s,
                    None => {
                        let _ = writeln!(report, "no answer within 120 s");
                    }
                }
                let _ = writeln!(report, "status after asking: {}", status_name(now));
            }
            if cmd == "list" {
                if matches!(now, PHAuthorizationStatus::Authorized | PHAuthorizationStatus::Limited) {
                    let n = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
                    report.push_str(&list(n));
                } else {
                    let _ = writeln!(report, "no access: nothing listed");
                }
            }
        }
        other => {
            let _ = writeln!(report, "unknown command {other:?}: use status, request or list [N]");
        }
    }
    print!("{report}");
    let _ = std::fs::write(format!("/tmp/photokit-probe-{variant}.txt"), &report);
}
