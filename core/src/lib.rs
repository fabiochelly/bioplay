//! Cœur natif de bioplay : l'index de tous les sous-titres d'un fichier, pour la liste qui
//! défile et les sauts exacts d'un sous-titre à l'autre. Commun à Android (JNI) et au bureau.

mod mkv;
pub mod model;
mod reader;
pub mod text;

use std::fs::File;
use std::io;

pub use model::{Cue, Track};

/// Fichiers texte au-delà : ce n'est pas un sous-titre.
const TEXT_LIMIT: u64 = 64 << 20;

pub fn index(file: File) -> io::Result<Vec<Track>> {
    let mut r = reader::Reader::new(file)?;
    if r.size >= 4 && r.peek(4)? == [0x1A, 0x45, 0xDF, 0xA3] {
        return mkv::index(&mut r);
    }
    if r.size > TEXT_LIMIT {
        return Ok(Vec::new());
    }
    let bytes = r.read_all(TEXT_LIMIT)?;
    if bytes.iter().take(4096).any(|&b| b == 0) && !(bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF])) {
        return Ok(Vec::new()); // binaire (MP4, AVI…) : pas d'index, l'app se rabat sur mpv
    }
    Ok(text::index(&bytes))
}

/// Instants (secondes) des images clés d'un MKV, d'après son index : sauts rapides quand une
/// image clé tombe assez près de la cible. Vide pour les autres formats.
pub fn keyframes(file: File) -> Vec<f64> {
    let Ok(mut r) = reader::Reader::new(file) else { return Vec::new() };
    if r.size >= 4 && r.peek(4).is_ok_and(|b| b == [0x1A, 0x45, 0xDF, 0xA3]) {
        return mkv::keyframes(&mut r).unwrap_or_default();
    }
    Vec::new()
}

pub fn index_encoded(file: File) -> String {
    match std::panic::catch_unwind(move || index(file)) {
        Ok(Ok(mut tracks)) => model::encode(&mut tracks),
        _ => String::new(),
    }
}

#[cfg(target_os = "android")]
mod android {
    use jni::objects::JClass;
    use jni::sys::{jdoubleArray, jint, jstring};
    use jni::JNIEnv;
    use std::fs::File;
    use std::os::fd::FromRawFd;

    /// `Core.index(fd)` : prend possession du descripteur et le ferme.
    #[no_mangle]
    pub extern "system" fn Java_app_bioplay_Core_index(env: JNIEnv, _: JClass, fd: jint) -> jstring {
        let file = unsafe { File::from_raw_fd(fd) };
        let out = super::index_encoded(file);
        env.new_string(out).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
    }

    /// `Core.keyframes(fd)` : instants (s) des images clés d'après l'index du MKV (vide sinon) ;
    /// prend possession du descripteur et le ferme.
    #[no_mangle]
    pub extern "system" fn Java_app_bioplay_Core_keyframes(env: JNIEnv, _: JClass, fd: jint) -> jdoubleArray {
        let file = unsafe { File::from_raw_fd(fd) };
        let k = std::panic::catch_unwind(move || super::keyframes(file)).unwrap_or_default();
        match env.new_double_array(k.len() as i32) {
            Ok(a) => {
                let _ = env.set_double_array_region(&a, 0, &k);
                a.into_raw()
            }
            Err(_) => std::ptr::null_mut(),
        }
    }
}
