use crate::{executor, header};

use core::convert::TryFrom as _;
use parity_scale_codec::DecodeAll as _;

/// Grandpa configuration of a chain, as extracted from the genesis block.
///
/// The way a chain configures Grandpa is either:
///
/// - Stored at the predefined `:grandpa_authorities` key of the storage.
/// - Retreived by calling the `GrandpaApi_grandpa_authorities` function of the runtime.
///
/// The latter method is soft-deprecated in former of the former. Both methods are still
/// supported.
///
/// > **Note**: Pragmatically speaking, Polkadot, Westend, and any newer chain use the former
/// >           method. Kusama only supports the latter.
///
#[derive(Debug, Clone)]
pub struct GrandpaGenesisConfiguration {
    /// Authorities of the authorities set 0. These are the authorities that finalize block #1.
    pub initial_authorities: Vec<header::GrandpaAuthority>,
}

type ConfigScaleEncoding = Vec<([u8; 32], u64)>;

impl GrandpaGenesisConfiguration {
    /// Retrieves the configuration from the storage of the genesis block.
    ///
    /// Must be passed a closure that returns the storage value corresponding to the given key in
    /// the genesis block storage.
    pub fn from_genesis_storage(
        mut genesis_storage_access: impl FnMut(&[u8]) -> Option<Vec<u8>>,
    ) -> Result<Self, FromGenesisStorageError> {
        let encoded_list = if let Some(mut list) = genesis_storage_access(b":grandpa_authorities") {
            // When in the storage, the encoded list of authorities starts with a version number.
            if list.first() != Some(&1) {
                return Err(FromGenesisStorageError::UnknownEncodingVersionNumber);
            }
            list.remove(0);
            list
        } else {
            let wasm_code =
                genesis_storage_access(b":code").ok_or(FromGenesisStorageError::RuntimeNotFound)?;
            let vm = executor::WasmVmPrototype::new(&wasm_code)
                .map_err(FromVmPrototypeError::VmInitialization)
                .map_err(FromGenesisStorageError::VmError)?;
            Self::from_virtual_machine_prototype(vm, genesis_storage_access)
                .map_err(FromGenesisStorageError::VmError)?
        };

        let decoded = match ConfigScaleEncoding::decode_all(&encoded_list) {
            Ok(cfg) => cfg,
            Err(err) => return Err(FromGenesisStorageError::OutputDecode(err)),
        };

        let initial_authorities = decoded
            .into_iter()
            .map(|(public_key, weight)| header::GrandpaAuthority { public_key, weight })
            .collect();

        Ok(GrandpaGenesisConfiguration {
            initial_authorities,
        })
    }

    fn from_virtual_machine_prototype(
        vm: executor::WasmVmPrototype,
        mut genesis_storage_access: impl FnMut(&[u8]) -> Option<Vec<u8>>,
    ) -> Result<Vec<u8>, FromVmPrototypeError> {
        // TODO: DRY with the babe config; put a helper in the executor module
        let mut vm = vm
            .run_no_param("GrandpaApi_grandpa_authorities")
            .map_err(FromVmPrototypeError::VmInitialization)?;

        Ok(loop {
            match vm.state() {
                executor::State::ReadyToRun(r) => r.run(),
                executor::State::Finished(data) => {
                    break data.to_owned();
                }
                executor::State::Trapped => return Err(FromVmPrototypeError::Trapped),

                executor::State::ExternalStorageGet {
                    storage_key,
                    offset,
                    max_size,
                    resolve,
                } => {
                    let mut value = genesis_storage_access(storage_key);

                    // TODO: maybe this could be a utility function in `executor`
                    if let Some(value) = &mut value {
                        if usize::try_from(offset).unwrap() < value.len() {
                            *value = value[usize::try_from(offset).unwrap()..].to_vec();
                            if usize::try_from(max_size).unwrap() < value.len() {
                                *value = value[..usize::try_from(max_size).unwrap()].to_vec();
                            }
                        } else {
                            *value = Vec::new();
                        }
                    }

                    resolve.finish_call(value);
                }

                _ => return Err(FromVmPrototypeError::ExternalityNotAllowed),
            }
        })
    }
}

/// Error when retrieving the Grandpa configuration.
#[derive(Debug, derive_more::Display)]
pub enum FromGenesisStorageError {
    /// Runtime couldn't be found in the genesis storage.
    RuntimeNotFound,
    /// Version number of the encoded authorities list isn't recognized.
    UnknownEncodingVersionNumber,
    /// Error while decoding the SCALE-encoded list.
    OutputDecode(parity_scale_codec::Error),
    /// Error while executing the runtime.
    VmError(FromVmPrototypeError),
}

/// Error when retrieving the Grandpa configuration.
#[derive(Debug, derive_more::Display)]
pub enum FromVmPrototypeError {
    /// Error when initializing the virtual machine.
    VmInitialization(executor::NewErr),
    /// Crash while running the virtual machine.
    Trapped,
    /// Virtual machine tried to call an externality that isn't valid in this context.
    ExternalityNotAllowed,
}
