use crate::active_anchor_stats::IIDomain;
use crate::{IC0_APP_ORIGIN, INTERNETCOMPUTER_ORG_ORIGIN};
use candid::{CandidType, Deserialize, Principal};
use internet_identity_interface::archive::types::DeviceDataWithoutAlias;
use internet_identity_interface::internet_identity::types::*;
use std::{fmt, iter};

#[cfg(test)]
mod tests;

/// Internal representation of the anchor.
/// The anchor has limited visibility for the constructor to make sure it is loaded from storage.
/// The devices can only be modified by the exposed functions which keeps invariant checking local
/// to this module.
#[derive(Clone, Debug, Default, CandidType, Deserialize, Eq, PartialEq)]
pub struct Anchor {
    devices: Vec<Device>,
}

impl Device {
    /// Applies the values of `device_data` to self while leaving the other fields intact.
    pub fn apply_device_data(&mut self, device_data: DeviceData) {
        self.pubkey = device_data.pubkey;
        self.alias = device_data.alias;
        self.credential_id = device_data.credential_id;
        self.purpose = device_data.purpose;
        self.key_type = device_data.key_type;
        self.protection = device_data.protection;
        self.origin = device_data.origin;
    }
}

impl From<DeviceData> for Device {
    fn from(device_data: DeviceData) -> Self {
        Self {
            pubkey: device_data.pubkey,
            alias: device_data.alias,
            credential_id: device_data.credential_id,
            purpose: device_data.purpose,
            key_type: device_data.key_type,
            protection: device_data.protection,
            origin: device_data.origin,
            last_usage_timestamp: None,
        }
    }
}

impl From<Device> for DeviceData {
    fn from(device: Device) -> Self {
        Self {
            pubkey: device.pubkey,
            alias: device.alias,
            credential_id: device.credential_id,
            purpose: device.purpose,
            key_type: device.key_type,
            protection: device.protection,
            origin: device.origin,
        }
    }
}

impl From<Device> for DeviceWithUsage {
    fn from(device: Device) -> Self {
        Self {
            pubkey: device.pubkey,
            alias: device.alias,
            credential_id: device.credential_id,
            purpose: device.purpose,
            key_type: device.key_type,
            protection: device.protection,
            origin: device.origin,
            last_usage: device.last_usage_timestamp,
        }
    }
}

impl From<Device> for DeviceDataWithoutAlias {
    fn from(device_data: Device) -> Self {
        Self {
            pubkey: device_data.pubkey,
            credential_id: device_data.credential_id,
            purpose: device_data.purpose,
            key_type: device_data.key_type,
            protection: device_data.protection,
            origin: device_data.origin,
        }
    }
}

impl Anchor {
    /// Creation of new anchors is restricted in order to make sure that the device checks are
    /// not accidentally bypassed.
    pub(super) fn new() -> Anchor {
        Self { devices: vec![] }
    }

    pub fn add_device(&mut self, device: Device) -> Result<(), AnchorError> {
        if self.devices.iter().any(|e| e.pubkey == device.pubkey) {
            return Err(AnchorError::DuplicateDevice {
                device_key: device.pubkey,
            });
        }
        check_device_invariants(&device)?;
        check_anchor_invariants(&self.devices.iter().chain(iter::once(&device)).collect())?;
        self.devices.push(device);
        Ok(())
    }

    /// Removes a device from this anchor.
    /// **Note:** Does not check invariants, based on the assumption that no invariant can be
    /// violated by removing a device. See also the documentation on
    /// [check_invariants](Anchor::check_invariants).
    pub fn remove_device(&mut self, device_key: &DeviceKey) -> Result<(), AnchorError> {
        let index = self.device_index(device_key)?;
        check_mutation_allowed(&self.devices[index])?;

        self.devices.remove(index);

        // We do _not_ check invariants here, because there might be anchors that do not fulfill
        // the invariants still stored in stable memory (e.g. anchors with multiple recovery phrases).
        // By allowing the removal of devices on such anchors, they can be made conforming to the invariants
        // by removing devices.
        Ok(())
    }

    pub fn modify_device(
        &mut self,
        device_key: &DeviceKey,
        modified_device: Device,
    ) -> Result<(), AnchorError> {
        if device_key != &modified_device.pubkey {
            return Err(AnchorError::CannotModifyDeviceKey);
        }
        check_device_invariants(&modified_device)?;
        let index = self.device_index(device_key)?;
        check_mutation_allowed(&self.devices[index])?;
        check_anchor_invariants(
            &self
                .devices
                .iter()
                // filter out the device before modification
                .filter(|e| e.pubkey != device_key)
                // append the device with modification
                .chain(iter::once(&modified_device))
                .collect(),
        )?;

        self.devices[index] = modified_device;
        Ok(())
    }

    fn device_index(&self, device_key: &DeviceKey) -> Result<usize, AnchorError> {
        let Some(index) = self.devices.iter().position(|e| e.pubkey == device_key) else {
            return Err(AnchorError::NotFound {device_key: device_key.clone()});
        };
        Ok(index)
    }

    /// Returns a reference to the device given the key.
    pub fn device(&self, device_key: &DeviceKey) -> Option<&Device> {
        self.devices.iter().find(|e| e.pubkey == device_key)
    }

    /// Returns a reference to the list of devices.
    pub fn devices(&self) -> &Vec<Device> {
        &self.devices
    }

    /// Consumes self and exposes the devices.
    pub fn into_devices(self) -> Vec<Device> {
        self.devices
    }

    /// Sets the timestamp on the given device.
    /// **Note:** Does not check invariants, based on the assumption that no invariant can be
    /// violated by changing the last usage timestamp on a device. See also the documentation on
    /// [check_invariants](Anchor::check_invariants).
    pub fn set_device_usage_timestamp(
        &mut self,
        device_key: &DeviceKey,
        time: Timestamp,
    ) -> Result<(), AnchorError> {
        let Some(device) = self.devices.iter_mut().find(|d| d.pubkey == device_key) else {
            return Err(AnchorError::NotFound { device_key: device_key.clone() })
        };
        device.last_usage_timestamp = Some(time);
        Ok(())
    }

    /// Returns the timestamp of the last known activity, if any.
    pub fn last_activity(&self) -> Option<Timestamp> {
        let mut timestamps: Vec<Option<Timestamp>> = self
            .devices
            .iter()
            .map(|d| d.last_usage_timestamp)
            .collect();
        timestamps.sort_unstable();
        timestamps.pop().unwrap_or_default()
    }

    /// Returns information about the domains this anchor was active on since the given timestamp.
    /// Activity on unknown / other domain will be dropped if there is also activity on an II domain
    /// for the following reasons:
    /// * no information is most likely caused by the device having been added before we started
    ///   collecting domain information
    /// * combinations of an unknown domain and an II domain shows that the anchor is at least partially
    ///   active on the II domain (but also does non-standard / unsupported things to their anchor).
    ///   If we are interested in this user group, we might consider extending this function to give
    ///   them their own [DomainActivity] value.
    pub fn domain_activity_since(&self, timestamp: Timestamp) -> DomainActivity {
        #[derive(Default)]
        struct Accumulator {
            ic0_app: bool,
            internet_computer_org: bool,
            non_ii: bool,
        }

        let result = self
            .devices
            .iter()
            // filter devices with no activity
            .filter(|d| {
                d.last_usage_timestamp
                    .map(|t| t >= timestamp)
                    .unwrap_or(false)
            })
            // assign domain activity
            .fold(Accumulator::default(), |mut acc, device| {
                let Some(ref origin) = device.origin else {
                    acc.non_ii = true;
                    return acc;
                };
                match origin.as_str() {
                    IC0_APP_ORIGIN => acc.ic0_app = true,
                    INTERNETCOMPUTER_ORG_ORIGIN => acc.internet_computer_org = true,
                    _ => acc.non_ii = true,
                };
                acc
            });

        // Activity on other domains is discarded if there is also activity on an II domain.
        // The reason is that II might not have complete information since domain information was
        // only introduced recently.
        match (result.ic0_app, result.internet_computer_org, result.non_ii) {
            (true, true, _) => DomainActivity::BothIIDomains,
            (true, false, _) => DomainActivity::Ic0App,
            (false, true, _) => DomainActivity::InternetComputerOrg,
            (false, false, true) => DomainActivity::NonIIDomain,
            (false, false, false) => DomainActivity::None,
        }
    }
}

/// Possible outcomes of domain bound activity for an anchor since a specific timestamp.
pub enum DomainActivity {
    // no activity at all
    None,
    // only active on non-ii domains
    NonIIDomain,
    // only active on the identity.ic0.app domain
    Ic0App,
    // only active on the identity.internetcomputer.org domain
    InternetComputerOrg,
    // activity on both identity.ic0.app and identity.internetcomputer.org
    BothIIDomains,
}

/// This is an internal version of `DeviceData` useful to provide a
/// backwards compatible level between device data stored in stable memory.
/// It is similar to `DeviceDataInternal` but with redundant options removed
/// (which is possible due to the stable memory candid schema migration).
#[derive(Clone, Debug, CandidType, Deserialize, Eq, PartialEq)]
pub struct Device {
    pub pubkey: DeviceKey,
    pub alias: String,
    pub credential_id: Option<CredentialId>,
    pub purpose: Purpose,
    pub key_type: KeyType,
    pub protection: DeviceProtection,
    pub origin: Option<String>,
    pub last_usage_timestamp: Option<Timestamp>,
}

impl Device {
    pub fn variable_fields_len(&self) -> usize {
        self.alias.len()
            + self.pubkey.len()
            + self.credential_id.as_ref().map(|id| id.len()).unwrap_or(0)
            + self.origin.as_ref().map(|origin| origin.len()).unwrap_or(0)
    }

    pub fn ii_domain(&self) -> Option<IIDomain> {
        self.origin
            .as_ref()
            .and_then(|origin| IIDomain::try_from(origin.as_str()).ok())
    }
}

fn check_mutation_allowed(device: &Device) -> Result<(), AnchorError> {
    match device.protection {
        DeviceProtection::Unprotected => (),
        DeviceProtection::Protected => {
            if caller() != Principal::self_authenticating(&device.pubkey) {
                return Err(AnchorError::MutationNotAllowed {
                    actual_principal: caller(),
                    authorized_principal: Principal::self_authenticating(&device.pubkey),
                });
            }
        }
    };
    Ok(())
}

#[cfg(not(test))]
fn caller() -> Principal {
    ic_cdk::caller()
}

/// This is required because [ic_cdk::caller()] traps when executed in a non-canister environment.
#[cfg(test)]
fn caller() -> Principal {
    tests::test_caller()
}

/// This checks anchor invariants, in particular:
///   * Max number of devices
///   * Sum of sizes of all variable length fields does not exceed limit
///   * There can only be one recovery phrase
///
/// **Important:**
/// Do **not** introduce new invariants that can be violated by _removing_ devices. The reason
/// is that there might still be devices in stable memory that do not fulfill the invariants.
/// In order to not break those anchors, they need to have a path back to satisfying the invariants.
/// To allow that transition, [remove_device](Anchor::remove_device) does _not_ check the invariants based on the assumption
/// that the state of an anchor cannot get worse by removing a device.
fn check_anchor_invariants(devices: &Vec<&Device>) -> Result<(), AnchorError> {
    /// The number of devices is limited. The front-end limits the devices further
    /// by only allowing 8 devices with purpose `authentication` to make sure there is always
    /// a slot for the recovery devices.
    /// Note however, that a free device slot does not guarantee that it will fit the the anchor
    /// due to the `VARIABLE_FIELDS_LIMIT`.
    const MAX_DEVICES_PER_ANCHOR: usize = 10;

    /// Single devices can use up to 564 bytes for the variable length fields alone.
    /// In order to not give away all the anchor space to the device vector, we limit the sum of the
    /// size of all variable fields of all devices. This ensures that we have the flexibility to expand
    /// or change anchors in the future.
    /// The value 2048 was chosen because it is the max anchor size before the stable memory migration.
    /// This means that all pre-existing anchors are below this limit. And after the migration, the
    /// candid encoded `vec devices` will stay far below 4KB in size (testing showed anchors of
    /// ~2500 bytes).
    const VARIABLE_FIELDS_LIMIT: usize = 2348;

    if devices.len() > MAX_DEVICES_PER_ANCHOR {
        return Err(AnchorError::TooManyDevices {
            num_devices: devices.len(),
            limit: MAX_DEVICES_PER_ANCHOR,
        });
    }

    let existing_variable_size: usize = devices
        .iter()
        // filter out the device being checked to not count it twice in case of update operations
        .map(|device| device.variable_fields_len())
        .sum();

    if existing_variable_size > VARIABLE_FIELDS_LIMIT {
        return Err(AnchorError::CumulativeDataLimitExceeded {
            length: existing_variable_size,
            limit: VARIABLE_FIELDS_LIMIT,
        });
    }

    // check that there is only a single recovery phrase
    if devices
        .iter()
        .filter(|device| device.key_type == KeyType::SeedPhrase)
        .count()
        > 1
    {
        return Err(AnchorError::MultipleRecoveryPhrases);
    }

    Ok(())
}

/// This checks device invariants, in particular:
///   * Sizes of various fields do not exceed limits
///   * Only recovery phrases can be protected
///
///  NOTE: while in the future we may lift this restriction, for now we do ensure that
///  protected devices are limited to recovery phrases, which the webapp expects.
fn check_device_invariants(device: &Device) -> Result<(), AnchorError> {
    check_device_limits(device)?;

    if device.protection == DeviceProtection::Protected && device.key_type != KeyType::SeedPhrase {
        return Err(AnchorError::InvalidDeviceProtection {
            key_type: device.key_type.clone(),
        });
    }
    Ok(())
}

fn check_device_limits(device: &Device) -> Result<(), AnchorError> {
    const ORIGIN_LEN_LIMIT: usize = 50;
    const ALIAS_LEN_LIMIT: usize = 64;
    const PK_LEN_LIMIT: usize = 300;
    const CREDENTIAL_ID_LEN_LIMIT: usize = 200;

    let n = device.alias.len();
    if n > ALIAS_LEN_LIMIT {
        return Err(AnchorError::DeviceLimitExceeded {
            field: "alias".to_string(),
            length: n,
            limit: ALIAS_LEN_LIMIT,
        });
    }

    let n = device.pubkey.len();
    if n > PK_LEN_LIMIT {
        return Err(AnchorError::DeviceLimitExceeded {
            field: "pubkey".to_string(),
            length: n,
            limit: PK_LEN_LIMIT,
        });
    }

    let n = device
        .credential_id
        .as_ref()
        .map(|bytes| bytes.len())
        .unwrap_or_default();
    if n > CREDENTIAL_ID_LEN_LIMIT {
        return Err(AnchorError::DeviceLimitExceeded {
            field: "credential_id".to_string(),
            length: n,
            limit: CREDENTIAL_ID_LEN_LIMIT,
        });
    }

    let n = device
        .origin
        .as_ref()
        .map(|bytes| bytes.len())
        .unwrap_or_default();
    if n > ORIGIN_LEN_LIMIT {
        return Err(AnchorError::DeviceLimitExceeded {
            field: "origin".to_string(),
            length: n,
            limit: ORIGIN_LEN_LIMIT,
        });
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
pub enum AnchorError {
    TooManyDevices {
        limit: usize,
        num_devices: usize,
    },
    DeviceLimitExceeded {
        field: String,
        length: usize,
        limit: usize,
    },
    CumulativeDataLimitExceeded {
        length: usize,
        limit: usize,
    },
    InvalidDeviceProtection {
        key_type: KeyType,
    },
    MutationNotAllowed {
        authorized_principal: Principal,
        actual_principal: Principal,
    },
    MultipleRecoveryPhrases,
    CannotModifyDeviceKey,
    NotFound {
        device_key: DeviceKey,
    },
    DuplicateDevice {
        device_key: DeviceKey,
    },
}

impl fmt::Display for AnchorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnchorError::TooManyDevices { num_devices, limit } => write!(
                f,
                "Anchor device limit exceeded: num devices {num_devices}, limit {limit}"
            ),
            AnchorError::DeviceLimitExceeded {
                field,
                length,
                limit,
            } => write!(
                f,
                "{field} limit exceeded: length {length}, limit {limit}"
            ),
            AnchorError::CumulativeDataLimitExceeded { length, limit } => write!(
                f,
                "Cumulative size of variable sized fields exceeds limit: length {length}, limit {limit}. Either use shorter aliases or remove an existing device."
            ),
            AnchorError::InvalidDeviceProtection { key_type } => write!(
                f,
                "Only recovery phrases can be locked but key type is {key_type:?}"
            ),
            AnchorError::MutationNotAllowed { actual_principal, authorized_principal } => write!(
                f,
                "Device is locked. Must be authenticated with this device to mutate: authorized principal {authorized_principal}, actual principal {actual_principal}"
            ),
            AnchorError::MultipleRecoveryPhrases => write!(f, "There is already a recovery phrase and only one is allowed."),
            AnchorError::CannotModifyDeviceKey => write!(f, "Device key cannot be updated."),
            AnchorError::NotFound { device_key } => write!(f, "Device with key {} not found.", hex::encode(device_key)),
            AnchorError::DuplicateDevice { device_key } => write!(f, "Device with key {} already exists on this anchor.", hex::encode(device_key)),
        }
    }
}
