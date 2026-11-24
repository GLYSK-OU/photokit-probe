//! Edit-back probe: save an edit into Photos the way an editing app does (rendered JPEG plus
//! `PHAdjustmentData`), so Photos shows it as edited and "Revert to Original" restores it.
//!
//! Safety rule: the only asset it may touch is the single image in the user album named exactly
//! `LC-probe`. No such album, several, or anything but exactly one image in it: refused, nothing
//! changed. The edit is a plain black-and-white conversion of a JPEG or PNG original.
//!
//! Commands: `inspect` (read-only: the target, `hasAdjustments`, its resources) and `edit`.

use std::fmt::Write as _;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{AllocAnyThread, Message};
use objc2_foundation::{NSArray, NSData, NSDate, NSDictionary, NSError, NSRunLoop, NSString};
use objc2_photos::{
    PHAccessLevel, PHAdjustmentData, PHAsset, PHAssetChangeRequest, PHAssetCollection, PHAssetCollectionSubtype, PHAssetCollectionType,
    PHAssetMediaType, PHAssetResource, PHAuthorizationStatus, PHContentEditingInput, PHContentEditingInputRequestOptions,
    PHContentEditingOutput, PHPhotoLibrary,
};

pub const ALBUM: &str = "LC-probe";
const FORMAT: &str = "eu.glysk.photokit-probe";

/// Wait for a value while running the main run loop: PhotoKit delivers some completion handlers on
/// the main queue, which a blocked main thread would never run.
fn wait<T>(rx: &mpsc::Receiver<T>, secs: u64) -> Option<T> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Ok(v) = rx.try_recv() {
            return Some(v);
        }
        let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
        NSRunLoop::currentRunLoop().runUntilDate(&until);
        std::thread::sleep(Duration::from_millis(5));
    }
    rx.try_recv().ok()
}

fn resource_type(t: isize) -> &'static str {
    match t {
        1 => "photo",
        2 => "video",
        4 => "alternatePhoto",
        5 => "fullSizePhoto (rendered edit)",
        7 => "adjustmentData (edit recipe)",
        8 => "adjustmentBasePhoto",
        9 => "pairedVideo",
        _ => "other",
    }
}

/// The one asset this probe may change, or why there is none.
pub fn target() -> Result<Retained<PHAsset>, String> {
    let albums = unsafe { PHAssetCollection::fetchAssetCollectionsWithType_subtype_options(PHAssetCollectionType::Album, PHAssetCollectionSubtype::Any, None) };
    let mut found = Vec::new();
    for i in 0..unsafe { albums.count() } {
        let album = unsafe { albums.objectAtIndex(i) };
        if unsafe { album.localizedTitle() }.is_some_and(|t| t.to_string() == ALBUM) {
            found.push(album);
        }
    }
    if found.len() != 1 {
        return Err(format!("need exactly one album named {ALBUM:?}, found {}", found.len()));
    }
    let assets = unsafe { PHAsset::fetchAssetsInAssetCollection_options(&found[0], None) };
    let n = unsafe { assets.count() };
    if n != 1 {
        return Err(format!("album {ALBUM:?} must hold exactly one photo, it holds {n}"));
    }
    let asset = unsafe { assets.objectAtIndex(0) };
    if unsafe { asset.mediaType() } != PHAssetMediaType::Image {
        return Err(format!("the item in {ALBUM:?} isn't a photo"));
    }
    Ok(asset)
}

fn refetch(id: &str) -> Option<Retained<PHAsset>> {
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    let r = unsafe { PHAsset::fetchAssetsWithLocalIdentifiers_options(&ids, None) };
    (unsafe { r.count() } == 1).then(|| unsafe { r.objectAtIndex(0) })
}

/// Identifier, edited flag and resources of an asset.
#[allow(deprecated)]
pub fn describe(asset: &PHAsset) -> String {
    let mut out = String::new();
    let id = unsafe { asset.localIdentifier() }.to_string();
    let edited = unsafe { asset.hasAdjustments() };
    let modified = unsafe { asset.modificationDate() }.map(|d| d.description().to_string()).unwrap_or_else(|| "-".into());
    let (w, h) = unsafe { (asset.pixelWidth(), asset.pixelHeight()) };
    let _ = writeln!(out, "asset {id} {w}x{h} hasAdjustments={edited} modified={modified}");
    for r in unsafe { PHAssetResource::assetResourcesForAsset(asset) }.iter() {
        let t = unsafe { r.r#type() }.0;
        let name = unsafe { r.originalFilename() }.to_string();
        let uti = unsafe { r.uniformTypeIdentifier() }.to_string();
        let _ = writeln!(out, "  resource {t} {}: {name} ({uti})", resource_type(t));
    }
    out
}

fn ensure_access(out: &mut String) -> bool {
    let mut s = unsafe { PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite) };
    if s == PHAuthorizationStatus::NotDetermined {
        let _ = writeln!(out, "asking macOS for Photos access (a prompt may appear)...");
        s = crate::request(Duration::from_secs(120)).unwrap_or(s);
    }
    let _ = writeln!(out, "access: {}", crate::status_name(s));
    matches!(s, PHAuthorizationStatus::Authorized | PHAuthorizationStatus::Limited)
}

fn edit(asset: &Retained<PHAsset>, out: &mut String) -> Result<(), String> {
    // 1. The original to edit (downloaded from iCloud if needed).
    let options = unsafe { PHContentEditingInputRequestOptions::new() };
    unsafe { options.setNetworkAccessAllowed(true) };
    let (tx, rx) = mpsc::channel::<Option<Retained<PHContentEditingInput>>>();
    let handler = RcBlock::new(move |input: *mut PHContentEditingInput, _info: std::ptr::NonNull<NSDictionary>| {
        let _ = tx.send(unsafe { input.as_ref() }.map(|i| i.retain()));
    });
    unsafe { asset.requestContentEditingInputWithOptions_completionHandler(Some(&options), &handler) };
    let input = wait(&rx, 180).flatten().ok_or("Photos gave no editing input (timeout or refused)")?;
    let src = unsafe { input.fullSizeImageURL() }
        .and_then(|u| unsafe { u.path() })
        .map(|p| p.to_string())
        .ok_or("editing input has no full-size image file")?;
    let _ = writeln!(out, "original for editing: {src}");

    // 2. Render the edit: black and white, JPEG.
    let bytes = std::fs::read(&src).map_err(|e| format!("can't read the original: {e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("can't decode the original (use a JPEG or PNG photo): {e}"))?;
    let gray = image::DynamicImage::ImageLuma8(img.to_luma8()).to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 92)
        .encode_image(&gray)
        .map_err(|e| format!("can't encode the edit: {e}"))?;

    // 3. The editing output: rendered file plus the recipe Photos keeps for revert and re-edit.
    let output = unsafe { PHContentEditingOutput::initWithContentEditingInput(PHContentEditingOutput::alloc(), &input) };
    let recipe = NSData::with_bytes(br#"{"edit":"monochrome","by":"photokit-probe"}"#);
    let adjustment = unsafe {
        PHAdjustmentData::initWithFormatIdentifier_formatVersion_data(PHAdjustmentData::alloc(), &NSString::from_str(FORMAT), &NSString::from_str("1"), &recipe)
    };
    unsafe { output.setAdjustmentData(Some(&adjustment)) };
    let dst = unsafe { output.renderedContentURL() }
        .path()
        .map(|p| p.to_string())
        .ok_or("rendered content URL has no path")?;
    std::fs::write(&dst, &jpeg).map_err(|e| format!("can't write the rendered edit: {e}"))?;
    let _ = writeln!(out, "rendered edit: {} bytes", jpeg.len());

    // 4. Commit it to the library.
    let target = asset.clone();
    let out_for_change = output.clone();
    let change = RcBlock::new(move || {
        let request = unsafe { PHAssetChangeRequest::changeRequestForAsset(&target) };
        unsafe { request.setContentEditingOutput(Some(&out_for_change)) };
    });
    let (tx, rx) = mpsc::channel::<(bool, Option<String>)>();
    let done = RcBlock::new(move |ok: Bool, err: *mut NSError| {
        let why = unsafe { err.as_ref() }.map(|e| format!("{} (code {})", e.localizedDescription(), e.code()));
        let _ = tx.send((ok.as_bool(), why));
    });
    let _ = writeln!(out, "saving into Photos (macOS may ask to allow the change)...");
    let library = unsafe { PHPhotoLibrary::sharedPhotoLibrary() };
    unsafe { library.performChanges_completionHandler(RcBlock::as_ptr(&change), Some(&done)) };
    match wait(&rx, 300) {
        Some((true, _)) => Ok(()),
        Some((false, why)) => Err(format!("Photos refused the change: {}", why.unwrap_or_else(|| "no reason given".into()))),
        None => Err("no answer from Photos within 300 s".into()),
    }
}

/// `inspect` or `edit`; prints the report and copies it to /tmp/photokit-edit.txt.
pub fn run(args: &[String]) {
    let mut out = String::new();
    let _ = writeln!(out, "photokit-edit {}", env!("CARGO_PKG_VERSION"));
    let cmd = args.first().map(String::as_str).unwrap_or("inspect");
    if !matches!(cmd, "inspect" | "edit") {
        let _ = writeln!(out, "unknown command {cmd:?}: use inspect or edit");
    } else if ensure_access(&mut out) {
        match target() {
            Err(why) => {
                let _ = writeln!(out, "refused, nothing changed: {why}");
            }
            Ok(asset) => {
                let _ = writeln!(out, "before:\n{}", describe(&asset));
                if cmd == "edit" {
                    let id = unsafe { asset.localIdentifier() }.to_string();
                    match edit(&asset, &mut out) {
                        Ok(()) => {
                            let _ = writeln!(out, "saved.");
                            if let Some(a) = refetch(&id) {
                                let _ = writeln!(out, "after:\n{}", describe(&a));
                            }
                        }
                        Err(why) => {
                            let _ = writeln!(out, "edit failed, library unchanged: {why}");
                        }
                    }
                }
            }
        }
    } else {
        let _ = writeln!(out, "no access: nothing done");
    }
    print!("{out}");
    let _ = std::fs::write("/tmp/photokit-edit.txt", &out);
}
