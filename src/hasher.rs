use std::io::Read;
use std::path::Path;
use crate::error::Result;
use crate::model::HashAlgo;

pub fn hash_file(path: &Path, algo: HashAlgo) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    match algo {
        HashAlgo::Md5 => {
            use md5::{Digest, Md5};
            let mut hasher = Md5::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        HashAlgo::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher)?;
            Ok(hasher.finalize().to_hex().to_string())
        }
    }
}

pub fn hash_file_blocking(path: &str, algo: HashAlgo) -> Result<String> {
    hash_file(Path::new(path), algo)
}
