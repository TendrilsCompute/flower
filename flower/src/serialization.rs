use camino::{Utf8Path, Utf8PathBuf};
use rmp_serde::{Serializer, config::BytesMode, decode, encode, from_slice};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
  fs,
  io::{self, AsyncWriteExt},
};

pub async fn write(thing: impl Serialize, path: &Utf8Path) -> SerializationResult<()> {
  let serialized = serialize(thing, path)?;

  fs::File::create(path)
    .await
    .map_err(io_err(path))?
    .write_all(&serialized)
    .await
    .map_err(io_err(path))?;
  Ok(())
}

pub async fn read<T: DeserializeOwned>(path: &Utf8Path) -> SerializationResult<T> {
  let serialized = fs::read(path).await.map_err(io_err(path))?;
  deserialize::<T>(&serialized, path)
}

pub fn serialize(thing: impl Serialize, path: &Utf8Path) -> SerializationResult<Vec<u8>> {
  let mut serialized = Vec::new();
  let mut serializer =
    Serializer::new(&mut serialized).with_struct_map().with_bytes(BytesMode::ForceAll);
  thing.serialize(&mut serializer).map_err(encode_err(path))?;
  Ok(serialized)
}

pub fn deserialize<T: DeserializeOwned>(serialized: &[u8], path: &Utf8Path) -> SerializationResult<T> {
  from_slice::<T>(serialized).map_err(decode_err(path))
}


#[derive(Debug)]
pub enum SerializationError {
  Io(Utf8PathBuf, io::Error),
  Encode(Utf8PathBuf, encode::Error),
  Decode(Utf8PathBuf, decode::Error),
}
pub fn io_err(path: &Utf8Path) -> impl FnOnce(io::Error) -> SerializationError {
  |err| SerializationError::Io(path.into(), err)
}
pub fn encode_err(path: &Utf8Path) -> impl FnOnce(encode::Error) -> SerializationError {
  |err| SerializationError::Encode(path.into(), err)
}
pub fn decode_err(path: &Utf8Path) -> impl FnOnce(decode::Error) -> SerializationError {
  |err| SerializationError::Decode(path.into(), err)
}

type SerializationResult<T> = Result<T, SerializationError>;
