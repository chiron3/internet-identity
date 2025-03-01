use crate::archive::{ArchiveData, ArchiveState, ArchiveStatusCache};
use crate::storage::anchor::Anchor;
use crate::storage::DEFAULT_RANGE_SIZE;
use crate::{Salt, Storage};
use candid::{CandidType, Deserialize, Principal};
use ic_cdk::api::time;
use ic_cdk::{call, trap};
use ic_certified_map::{Hash, RbTree};
use ic_stable_structures::DefaultMemoryImpl;
use internet_identity::signature_map::SignatureMap;
use internet_identity_interface::http_gateway::HeaderField;
use internet_identity_interface::internet_identity::types::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::time::Duration;

pub type Assets = HashMap<String, (Vec<HeaderField>, Vec<u8>)>;
pub type AssetHashes = RbTree<String, Hash>;

// Default value for max number of delegation origins to store in the list of latest used delegation origins
const MAX_NUM_DELEGATION_ORIGINS: u64 = 1000;

thread_local! {
    static STATE: State = State::default();
    static ASSETS: RefCell<Assets> = RefCell::new(HashMap::default());
}

pub struct TentativeDeviceRegistration {
    pub expiration: Timestamp,
    pub state: RegistrationState,
}

/// Registration state of new devices added using the two step device add flow
pub enum RegistrationState {
    DeviceRegistrationModeActive,
    DeviceTentativelyAdded {
        tentative_device: DeviceData,
        verification_code: DeviceVerificationCode,
        failed_attempts: FailedAttemptsCounter,
    },
}

#[derive(Default)]
pub struct UsageMetrics {
    // number of prepare_delegation calls since last upgrade
    pub delegation_counter: u64,
    // number of anchor operations (register, add, remove, update) since last upgrade
    pub anchor_operation_counter: u64,
}

// The challenges we store and check against
pub struct ChallengeInfo {
    pub created: Timestamp,
    pub chars: String,
}

pub type ChallengeKey = String;

// The user's attempt
#[derive(Clone, Debug, CandidType, Deserialize)]
pub struct ChallengeAttempt {
    pub chars: String,
    pub key: ChallengeKey,
}

// What we send the user
#[derive(Clone, Debug, CandidType, Deserialize)]
pub struct Challenge {
    pub png_base64: String,
    pub challenge_key: ChallengeKey,
}

#[derive(Clone, CandidType, Deserialize, Eq, PartialEq, Debug)]
pub struct PersistentState {
    // Information related to the archive
    pub archive_state: ArchiveState,
    // Amount of cycles that need to be attached when II creates a canister
    pub canister_creation_cycles_cost: u64,
    // Configuration for the rate limit on `register`, if any.
    pub registration_rate_limit: Option<RateLimitConfig>,
    // Daily and monthly active anchor statistics
    pub active_anchor_stats: Option<ActiveAnchorStatistics<ActiveAnchorCounter>>,
    // Daily and monthly active anchor statistics (filtered by domain)
    pub domain_active_anchor_stats: Option<ActiveAnchorStatistics<DomainActiveAnchorCounter>>,
    // Hashmap of last used delegation origins
    pub latest_delegation_origins: Option<HashMap<FrontendHostname, Timestamp>>,
    // Maximum number of latest delegation origins to store
    pub max_num_latest_delegation_origins: Option<u64>,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            archive_state: ArchiveState::default(),
            canister_creation_cycles_cost: 0,
            registration_rate_limit: None,
            active_anchor_stats: None,
            domain_active_anchor_stats: None,
            latest_delegation_origins: None,
            max_num_latest_delegation_origins: Some(MAX_NUM_DELEGATION_ORIGINS),
        }
    }
}

#[derive(Clone, Debug, CandidType, Deserialize)]
pub struct RateLimitState {
    // Number of tokens available for calls, where each call will deduct one token. If tokens reaches
    // 0 the rate limit will cancel the call.
    pub tokens: u64,
    // Timestamp from which `time_per_token_ns` (see RateLimitConfig) must have passed to
    // increment `tokens`.
    pub token_timestamp: Timestamp,
}

enum StorageState {
    Uninitialised,
    Initialised(Storage<DefaultMemoryImpl>),
}

struct State {
    storage_state: RefCell<StorageState>,
    sigs: RefCell<SignatureMap>,
    asset_hashes: RefCell<AssetHashes>,
    last_upgrade_timestamp: Cell<Timestamp>,
    // note: we COULD persist this through upgrades, although this is currently NOT persisted
    // through upgrades
    inflight_challenges: RefCell<HashMap<ChallengeKey, ChallengeInfo>>,
    // tentative device registrations, not persisted across updates
    // if an anchor number is present in this map then registration mode is active until expiration
    tentative_device_registrations: RefCell<HashMap<AnchorNumber, TentativeDeviceRegistration>>,
    // additional usage metrics, NOT persisted across updates (but probably should be in the future)
    usage_metrics: RefCell<UsageMetrics>,
    // State that is temporarily persisted in stable memory during upgrades using
    // pre- and post-upgrade hooks.
    // This must remain small as it is serialized and deserialized on pre- and post-upgrade.
    // Be careful when making changes here, as II needs to be able to update and roll back.
    persistent_state: RefCell<PersistentState>,
    // Cache of the archive status (to make unwanted calls to deploy_archive cheap to dismiss).
    archive_status_cache: RefCell<Option<ArchiveStatusCache>>,
    // Tracking data for the registration rate limit, if any. Not persisted across upgrades.
    registration_rate_limit: RefCell<Option<RateLimitState>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            storage_state: RefCell::new(StorageState::Uninitialised),
            sigs: RefCell::new(SignatureMap::default()),
            asset_hashes: RefCell::new(AssetHashes::default()),
            last_upgrade_timestamp: Cell::new(0),
            inflight_challenges: RefCell::new(HashMap::new()),
            tentative_device_registrations: RefCell::new(HashMap::new()),
            usage_metrics: RefCell::new(UsageMetrics::default()),
            persistent_state: RefCell::new(PersistentState::default()),
            archive_status_cache: RefCell::new(None),
            registration_rate_limit: RefCell::new(None),
        }
    }
}

// Checks if salt is empty and calls `init_salt` to set it.
pub async fn ensure_salt_set() {
    let salt = storage_borrow(|storage| storage.salt().cloned());
    if salt.is_none() {
        init_salt().await;
    }

    storage_borrow(|storage| {
        if storage.salt().is_none() {
            trap("Salt is not set. Try calling init_salt() to set it");
        }
    });
}

pub async fn init_salt() {
    storage_borrow(|storage| {
        if storage.salt().is_some() {
            trap("Salt already set");
        }
    });

    let res: Vec<u8> = match call(Principal::management_canister(), "raw_rand", ()).await {
        Ok((res,)) => res,
        Err((_, err)) => trap(&format!("failed to get salt: {err}")),
    };
    let salt: Salt = res[..].try_into().unwrap_or_else(|_| {
        trap(&format!(
            "expected raw randomness to be of length 32, got {}",
            res.len()
        ));
    });

    storage_borrow_mut(|storage| storage.update_salt(salt)); // update_salt() traps if salt has already been set
}

pub fn salt() -> [u8; 32] {
    storage_borrow(|storage| {
        storage
            .salt()
            .cloned()
            .unwrap_or_else(|| trap("Salt is not set. Try calling init_salt() to set it"))
    })
}

pub fn init_new() {
    const FIRST_ANCHOR_NUMBER: AnchorNumber = 10_000;
    let storage = Storage::new(
        (
            FIRST_ANCHOR_NUMBER,
            FIRST_ANCHOR_NUMBER.saturating_add(DEFAULT_RANGE_SIZE),
        ),
        DefaultMemoryImpl::default(),
    );
    storage_replace(storage);
}

pub fn init_from_stable_memory() {
    STATE.with(|s| {
        s.last_upgrade_timestamp.set(time());
    });
    match Storage::from_memory(DefaultMemoryImpl::default()) {
        Some(new_storage) => {
            storage_replace(new_storage);
        }
        None => {
            storage_borrow_mut(|storage| storage.flush());
        }
    }
}

pub fn save_persistent_state() {
    STATE.with(|s| {
        storage_borrow_mut(|storage| storage.write_persistent_state(&s.persistent_state.borrow()))
    })
}

pub fn load_persistent_state() {
    STATE.with(|s| {
        storage_borrow(|storage| match storage.read_persistent_state() {
            Ok(loaded_state) => *s.persistent_state.borrow_mut() = loaded_state,
            Err(err) => trap(&format!("failed to recover persistent state! Err: {err:?}")),
        })
    });

    // Initialize a sensible default for max_latest_delegation_origins
    // if it is not set in the persistent state.
    // This will allow us to later drop the opt and make the field u64.
    persistent_state_mut(|persistent_state| {
        persistent_state
            .max_num_latest_delegation_origins
            .get_or_insert(MAX_NUM_DELEGATION_ORIGINS);
    });
}

// helper methods to access / modify the state in a convenient way

pub fn anchor(anchor: AnchorNumber) -> Anchor {
    storage_borrow(|storage| {
        storage.read(anchor).unwrap_or_else(|err| {
            trap(&format!(
                "failed to read device data of user {anchor}: {err}"
            ))
        })
    })
}

pub fn archive_state() -> ArchiveState {
    STATE.with(|s| s.persistent_state.borrow().archive_state.clone())
}

pub fn archive_data_mut<R>(f: impl FnOnce(&mut ArchiveData) -> R) -> R {
    STATE.with(|s| {
        if let ArchiveState::Created { ref mut data, .. } =
            s.persistent_state.borrow_mut().archive_state
        {
            f(data)
        } else {
            trap("no archive deployed")
        }
    })
}

pub fn tentative_device_registrations<R>(
    f: impl FnOnce(&HashMap<AnchorNumber, TentativeDeviceRegistration>) -> R,
) -> R {
    STATE.with(|s| f(&s.tentative_device_registrations.borrow()))
}

pub fn tentative_device_registrations_mut<R>(
    f: impl FnOnce(&mut HashMap<AnchorNumber, TentativeDeviceRegistration>) -> R,
) -> R {
    STATE.with(|s| f(&mut s.tentative_device_registrations.borrow_mut()))
}

pub fn assets<R>(f: impl FnOnce(&Assets) -> R) -> R {
    ASSETS.with(|assets| f(&assets.borrow()))
}

pub fn assets_and_hashes_mut<R>(f: impl FnOnce(&mut Assets, &mut AssetHashes) -> R) -> R {
    ASSETS.with(|assets| {
        STATE.with(|s| f(&mut assets.borrow_mut(), &mut s.asset_hashes.borrow_mut()))
    })
}

pub fn asset_hashes_and_sigs<R>(f: impl FnOnce(&AssetHashes, &SignatureMap) -> R) -> R {
    STATE.with(|s| f(&s.asset_hashes.borrow(), &s.sigs.borrow()))
}

pub fn signature_map<R>(f: impl FnOnce(&SignatureMap) -> R) -> R {
    STATE.with(|s| f(&s.sigs.borrow()))
}

pub fn signature_map_mut<R>(f: impl FnOnce(&mut SignatureMap) -> R) -> R {
    STATE.with(|s| f(&mut s.sigs.borrow_mut()))
}

pub fn storage_borrow<R>(f: impl FnOnce(&Storage<DefaultMemoryImpl>) -> R) -> R {
    STATE.with(|s| match s.storage_state.borrow().deref() {
        StorageState::Uninitialised => trap("Storage not initialized."),
        StorageState::Initialised(storage) => f(storage),
    })
}

pub fn storage_borrow_mut<R>(f: impl FnOnce(&mut Storage<DefaultMemoryImpl>) -> R) -> R {
    STATE.with(|s| match s.storage_state.borrow_mut().deref_mut() {
        StorageState::Uninitialised => trap("Storage not initialized."),
        StorageState::Initialised(ref mut storage) => f(storage),
    })
}

pub fn storage_replace(storage: Storage<DefaultMemoryImpl>) {
    STATE.with(|s| s.storage_state.replace(StorageState::Initialised(storage)));
}

pub fn usage_metrics<R>(f: impl FnOnce(&UsageMetrics) -> R) -> R {
    STATE.with(|s| f(&s.usage_metrics.borrow()))
}

pub fn usage_metrics_mut<R>(f: impl FnOnce(&mut UsageMetrics) -> R) -> R {
    STATE.with(|s| f(&mut s.usage_metrics.borrow_mut()))
}

pub fn inflight_challenges<R>(f: impl FnOnce(&HashMap<ChallengeKey, ChallengeInfo>) -> R) -> R {
    STATE.with(|s| f(&s.inflight_challenges.borrow()))
}

pub fn inflight_challenges_mut<R>(
    f: impl FnOnce(&mut HashMap<ChallengeKey, ChallengeInfo>) -> R,
) -> R {
    STATE.with(|s| f(&mut s.inflight_challenges.borrow_mut()))
}

pub fn last_upgrade_timestamp() -> Timestamp {
    STATE.with(|s| s.last_upgrade_timestamp.get())
}

pub fn persistent_state<R>(f: impl FnOnce(&PersistentState) -> R) -> R {
    STATE.with(|s| f(&s.persistent_state.borrow()))
}

pub fn persistent_state_mut<R>(f: impl FnOnce(&mut PersistentState) -> R) -> R {
    STATE.with(|s| f(&mut s.persistent_state.borrow_mut()))
}

pub fn registration_rate_limit<R>(f: impl FnOnce(&Option<RateLimitState>) -> R) -> R {
    STATE.with(|s| f(&s.registration_rate_limit.borrow()))
}

pub fn registration_rate_limit_mut<R>(f: impl FnOnce(&mut Option<RateLimitState>) -> R) -> R {
    STATE.with(|s| f(&mut s.registration_rate_limit.borrow_mut()))
}

pub fn cached_archive_status() -> Option<ArchiveStatusCache> {
    STATE.with(|s| match *s.archive_status_cache.borrow() {
        None => None,
        Some(ref cached_status) => {
            // cache is outdated
            if time() - cached_status.timestamp > Duration::from_secs(60 * 60).as_nanos() as u64 {
                return None;
            }
            Some(cached_status.clone())
        }
    })
}

pub fn cache_archive_status(archive_status: ArchiveStatusCache) {
    STATE.with(|state| {
        *state.archive_status_cache.borrow_mut() = Some(archive_status);
    })
}

pub fn invalidate_archive_status_cache() {
    STATE.with(|state| {
        *state.archive_status_cache.borrow_mut() = None;
    })
}
