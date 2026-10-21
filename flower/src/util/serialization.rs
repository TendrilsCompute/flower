use brotli::{BrotliCompress, BrotliDecompress, enc::BrotliEncoderParams};
use camino::{Utf8Path, Utf8PathBuf};
use rmp_serde::{Serializer, config::BytesMode, decode, encode, from_slice};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::{fs, io};

pub async fn write(thing: impl Serialize, path: &Utf8Path) -> SerializationResult<()> {
  let compressed = compress(&serialize(thing, path)?).map_err(compression_err(path))?;
  fs::write(path, compressed).await.map_err(io_err(path))
}

pub async fn read<T: DeserializeOwned>(path: &Utf8Path) -> SerializationResult<T> {
  let compressed = fs::read(path).await.map_err(io_err(path))?;
  let serialized = decompress(&compressed).map_err(compression_err(path))?;
  deserialize::<T>(&serialized, path)
}

fn compress(mut data: &[u8]) -> io::Result<Vec<u8>> {
  let params = BrotliEncoderParams { quality: 6, ..Default::default() };
  let mut compressed = Vec::new();
  BrotliCompress(&mut data, &mut compressed, &params)?;
  Ok(compressed)
}

fn decompress(mut data: &[u8]) -> io::Result<Vec<u8>> {
  let mut decompressed = Vec::new();
  BrotliDecompress(&mut data, &mut decompressed)?;
  Ok(decompressed)
}

pub fn serialize(thing: impl Serialize, path: &Utf8Path) -> SerializationResult<Vec<u8>> {
  let mut serialized = Vec::new();
  let mut serializer =
    Serializer::new(&mut serialized).with_struct_map().with_bytes(BytesMode::ForceAll);
  thing.serialize(&mut serializer).map_err(encode_err(path))?;
  Ok(serialized)
}

pub fn deserialize<T: DeserializeOwned>(
  serialized: &[u8],
  path: &Utf8Path,
) -> SerializationResult<T> {
  from_slice::<T>(serialized).map_err(decode_err(path))
}

#[derive(Debug, Error)]
pub enum SerializationError {
  #[error("IO error ({0}): {1}")]
  Io(Utf8PathBuf, io::Error),
  #[error("Compression error ({0}): {1}")]
  Compression(Utf8PathBuf, io::Error),
  #[error("Encode error ({0}): {1}")]
  Encode(Utf8PathBuf, encode::Error),
  #[error("Decode error ({0}): {1}")]
  Decode(Utf8PathBuf, decode::Error),
}
pub fn io_err(path: &Utf8Path) -> impl FnOnce(io::Error) -> SerializationError {
  |err| SerializationError::Io(path.into(), err)
}
pub fn compression_err(path: &Utf8Path) -> impl FnOnce(io::Error) -> SerializationError {
  |err| SerializationError::Compression(path.into(), err)
}
pub fn encode_err(path: &Utf8Path) -> impl FnOnce(encode::Error) -> SerializationError {
  |err| SerializationError::Encode(path.into(), err)
}
pub fn decode_err(path: &Utf8Path) -> impl FnOnce(decode::Error) -> SerializationError {
  |err| SerializationError::Decode(path.into(), err)
}

type SerializationResult<T> = Result<T, SerializationError>;
