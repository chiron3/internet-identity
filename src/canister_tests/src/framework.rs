use crate::api;
use crate::api::http_request;
use candid::Principal;
use flate2::read::GzDecoder;
use flate2::{Compression, GzBuilder};
use ic_cdk::api::management_canister::main::CanisterId;
use ic_crypto_iccsa::types::SignatureBytes;
use ic_crypto_iccsa::{public_key_bytes_from_der, verify};
use ic_crypto_utils_threshold_sig_der::parse_threshold_sig_key_from_der;
use ic_test_state_machine_client::{CallError, ErrorCode, StateMachine};
use ic_types::crypto::Signable;
use ic_types::messages::Delegation;
use ic_types::Time;
use internet_identity_interface::archive::types::*;
use internet_identity_interface::http_gateway::{HeaderField, HttpRequest};
use internet_identity_interface::internet_identity::types::*;
use lazy_static::lazy_static;
use regex::Regex;
use serde_bytes::ByteBuf;
use sha2::Digest;
use sha2::Sha256;
use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::path;
use std::path::Path;
use std::time::{Duration, SystemTime};

/* The first few lines deal with actually getting the Wasm module(s) to test */

lazy_static! {
    /** The Wasm module for the current II build, i.e. the one we're testing */
    pub static ref II_WASM: Vec<u8> = {
        let def_path = path::PathBuf::from("..").join("..").join("internet_identity.wasm");
        let err = format!("
        Could not find Internet Identity Wasm module for current build.

        I will look for it at {:?}, and you can specify another path with the environment variable II_WASM (note that I run from {:?}).

        In order to build the Wasm module, please run the following command:
            II_DUMMY_CAPTCHA=1 ./scripts/build
        ", &def_path, &std::env::current_dir().map(|x| x.display().to_string()).unwrap_or_else(|_| "an unknown directory".to_string()));
        get_wasm_path("II_WASM".to_string(), &def_path).expect(&err)
    };

    /** The Wasm module for the current archive build, i.e. the one we're testing */
    pub static ref ARCHIVE_WASM: Vec<u8> = {
        let def_path = path::PathBuf::from("..").join("..").join("archive.wasm");
        let err = format!("
        Could not find Archive Wasm module for current build.

        I will look for it at {:?}, and you can specify another path with the environment variable ARCHIVE_WASM (note that I run from {:?}).

        In order to build the Wasm module, please run the following command:
            ./scripts/build --archive
        ", &def_path, &std::env::current_dir().map(|x| x.display().to_string()).unwrap_or_else(|_| "an unknown directory".to_string()));
        get_wasm_path("ARCHIVE_WASM".to_string(), &def_path).expect(&err)
    };

    /** The Wasm module for the _previous_ II build, or latest release, which is used when testing
     * upgrades and downgrades */
    pub static ref II_WASM_PREVIOUS: Vec<u8> = {
        let def_path = path::PathBuf::from("..").join("..").join("internet_identity_previous.wasm");
        let err = format!("
        Could not find Internet Identity Wasm module for previous build/latest release.

        I will look for it at {:?}, and you can specify another path with the environment variable II_WASM_PREVIOUS (note that I run from {:?}).

        In order to get the Wasm module, please run the following command:
            curl -SL https://github.com/dfinity/internet-identity/releases/latest/download/internet_identity_test.wasm -o internet_identity_previous.wasm
        ", &def_path, &std::env::current_dir().map(|x| x.display().to_string()).unwrap_or_else(|_| "an unknown directory".to_string()));
        get_wasm_path("II_WASM_PREVIOUS".to_string(), &def_path).expect(&err)
    };

        /** The Wasm module for the _previous_ archive build, or latest release, which is used when testing
            * upgrades and downgrades */
    pub static ref ARCHIVE_WASM_PREVIOUS: Vec<u8> = {
        let def_path = path::PathBuf::from("..").join("..").join("archive_previous.wasm");
        let err = format!("
        Could not find Archive Wasm module for previous build/latest release.

        I will look for it at {:?}, and you can specify another path with the environment variable ARCHIVE_WASM_PREVIOUS (note that I run from {:?}).

        In order to get the Wasm module, please run the following command:
            curl -SL https://github.com/dfinity/internet-identity/releases/latest/download/archive.wasm -o archive_previous.wasm
        ", &def_path, &std::env::current_dir().map(|x| x.display().to_string()).unwrap_or_else(|_| "an unknown directory".to_string()));
        get_wasm_path("ARCHIVE_WASM_PREVIOUS".to_string(), &def_path).expect(&err)
    };

    /** Empty WASM module (without any pre- and post-upgrade hooks. Useful to initialize a canister before loading a stable memory backup. */
    pub static ref EMPTY_WASM: Vec<u8> = vec![0, 0x61, 0x73, 0x6D, 1, 0, 0, 0];
}

/** Helper that returns the content of `default_path` if found, or None if the file does not exist.
 * The `env_var` environment variable is also read for custom location; if the variable is set
 * _but_ the Wasm module is not present, we simply panic (i.e. we don't return None)
 */
fn get_wasm_path(env_var: String, default_path: &path::PathBuf) -> Option<Vec<u8>> {
    match env::var_os(env_var.clone()) {
        None => {
            if !default_path.exists() {
                return None;
            }
            Some(
                std::fs::read(default_path)
                    .unwrap_or_else(|_| panic!("could not read Wasm module: {default_path:?}")),
            )
        }
        Some(path) => {
            let pathname: String = path
                .into_string()
                .unwrap_or_else(|_| panic!("Invalid string path for {env_var}"));
            let path = path::PathBuf::from(pathname.clone());
            if !path.exists() {
                panic!("Could not find {pathname}");
            }
            Some(
                std::fs::read(path.clone())
                    .unwrap_or_else(|_| panic!("could not read Wasm module: {path:?}")),
            )
        }
    }
}

/// The path to the state machine binary to run the tests with
pub static STATE_MACHINE_BINARY: &str = "../../ic-test-state-machine";

pub fn env() -> StateMachine {
    let path = match env::var_os("STATE_MACHINE_BINARY") {
        None => STATE_MACHINE_BINARY.to_string(),
        Some(path) => path
            .clone()
            .into_string()
            .unwrap_or_else(|_| panic!("Invalid string path for {path:?}")),
    };

    if !Path::new(&path).exists() {
        println!("
        Could not find state machine binary to run canister integration tests.

        I looked for it at {:?}. You can specify another path with the environment variable STATE_MACHINE_BINARY (note that I run from {:?}).

        Run the following command to get the binary:
            curl -sLO https://download.dfinity.systems/ic/$commit/binaries/$platform/ic-test-state-machine.gz
            gzip -d ic-test-state-machine.gz
            chmod +x ic-test-state-machine
        where $commit can be read from `.ic-commit` and $platform is 'x86_64-linux' for Linux and 'x86_64-darwin' for Intel/rosetta-enabled Darwin.
        ", &path, &env::current_dir().map(|x| x.display().to_string()).unwrap_or_else(|_| "an unknown directory".to_string()));
    }

    StateMachine::new(&path, false)
}

pub fn install_ii_canister(env: &StateMachine, wasm: Vec<u8>) -> CanisterId {
    install_ii_canister_with_arg(env, wasm, None)
}

pub fn install_ii_canister_with_arg(
    env: &StateMachine,
    wasm: Vec<u8>,
    arg: Option<InternetIdentityInit>,
) -> CanisterId {
    let byts = candid::encode_one(arg).expect("error encoding II installation arg as candid");
    let canister_id = env.create_canister();
    env.install_canister(canister_id, wasm, byts);
    canister_id
}

pub fn arg_with_wasm_hash(wasm: Vec<u8>) -> Option<InternetIdentityInit> {
    Some(InternetIdentityInit {
        assigned_user_number_range: None,
        archive_config: Some(ArchiveConfig {
            module_hash: archive_wasm_hash(&wasm),
            entries_buffer_limit: 10_000,
            polling_interval_ns: Duration::from_secs(1).as_nanos() as u64,
            entries_fetch_limit: 10,
        }),
        canister_creation_cycles_cost: Some(0),
        register_rate_limit: None,
        max_num_latest_delegation_origins: None,
    })
}

pub fn arg_with_rate_limit(rate_limit: RateLimitConfig) -> Option<InternetIdentityInit> {
    Some(InternetIdentityInit {
        assigned_user_number_range: None,
        archive_config: None,
        canister_creation_cycles_cost: None,
        register_rate_limit: Some(rate_limit),
        max_num_latest_delegation_origins: None,
    })
}

pub fn arg_with_anchor_range(
    anchor_range: (AnchorNumber, AnchorNumber),
) -> Option<InternetIdentityInit> {
    Some(InternetIdentityInit {
        assigned_user_number_range: Some(anchor_range),
        archive_config: None,
        canister_creation_cycles_cost: None,
        register_rate_limit: None,
        max_num_latest_delegation_origins: None,
    })
}

pub fn archive_wasm_hash(wasm: &Vec<u8>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(wasm);
    hasher.finalize().into()
}

pub fn upgrade_ii_canister(env: &StateMachine, canister_id: CanisterId, wasm: Vec<u8>) {
    upgrade_ii_canister_with_arg(env, canister_id, wasm, None).unwrap()
}

pub fn upgrade_ii_canister_with_arg(
    env: &StateMachine,
    canister_id: CanisterId,
    wasm: Vec<u8>,
    arg: Option<InternetIdentityInit>,
) -> Result<(), CallError> {
    let byts = candid::encode_one(arg).expect("error encoding II upgrade arg as candid");
    env.upgrade_canister(canister_id, wasm, byts)
}

/// Utility function to create compressed stable memory backups for use in backup tests.
pub fn save_compressed_stable_memory(
    env: &StateMachine,
    canister_id: CanisterId,
    path: &str,
    decompressed_name: &str,
) {
    let file = File::create(path).expect("Failed to create stable memory file");
    let mut encoder = GzBuilder::new()
        .filename(decompressed_name)
        .write(file, Compression::best());
    encoder
        .write_all(env.stable_memory(canister_id).as_slice())
        .unwrap();
    encoder.flush().unwrap();
    let mut file = encoder.finish().unwrap();
    file.flush().unwrap();
}

pub fn restore_compressed_stable_memory(env: &StateMachine, canister_id: CanisterId, path: &str) {
    let file = File::open(path).expect("Failed to open stable memory file");
    let mut decoder = GzDecoder::new(file);
    let mut buffer = vec![];
    decoder
        .read_to_end(&mut buffer)
        .expect("error while decoding stable memory file");
    env.set_stable_memory(canister_id, ByteBuf::from(buffer));
}

pub const PUBKEY_1: &str = "test";
pub const PUBKEY_2: &str = "some other key";
pub const RECOVERY_PUBKEY_1: &str = "recovery 1";
pub const RECOVERY_PUBKEY_2: &str = "recovery 2";

pub fn principal_1() -> Principal {
    Principal::self_authenticating(PUBKEY_1)
}

pub fn principal_2() -> Principal {
    Principal::self_authenticating(PUBKEY_2)
}
pub fn principal_recovery_1() -> Principal {
    Principal::self_authenticating(RECOVERY_PUBKEY_1)
}

pub fn principal_recovery_2() -> Principal {
    Principal::self_authenticating(RECOVERY_PUBKEY_2)
}

pub fn device_data_1() -> DeviceData {
    DeviceData {
        pubkey: ByteBuf::from(PUBKEY_1),
        alias: "My Device".to_string(),
        credential_id: Some(ByteBuf::from("credential id 1")),
        purpose: Purpose::Authentication,
        key_type: KeyType::Unknown,
        protection: DeviceProtection::Unprotected,
        origin: Some("https://identity.internetcomputer.org".to_string()),
    }
}

pub fn device_data_2() -> DeviceData {
    DeviceData {
        pubkey: ByteBuf::from(PUBKEY_2),
        alias: "My second device".to_string(),
        credential_id: Some(ByteBuf::from("credential id 2")),
        purpose: Purpose::Authentication,
        key_type: KeyType::Unknown,
        protection: DeviceProtection::Unprotected,
        origin: Some("https://identity.ic0.app".to_string()),
    }
}

pub fn max_size_device() -> DeviceData {
    DeviceData {
        pubkey: ByteBuf::from([255u8; 300]),
        alias: "a".repeat(64),
        credential_id: Some(ByteBuf::from([7u8; 200])),
        purpose: Purpose::Authentication,
        key_type: KeyType::Unknown,
        protection: DeviceProtection::Unprotected,
        origin: Some("https://rdmx6-jaaaa-aaaaa-aaadq-cai.foobar.icp0.io".to_string()),
    }
}

pub fn recovery_device_data_1() -> DeviceData {
    DeviceData {
        pubkey: ByteBuf::from(RECOVERY_PUBKEY_1),
        alias: "Recovery Phrase 1".to_string(),
        credential_id: None,
        purpose: Purpose::Recovery,
        key_type: KeyType::SeedPhrase,
        protection: DeviceProtection::Unprotected,
        origin: None,
    }
}

pub fn recovery_device_data_2() -> DeviceData {
    DeviceData {
        pubkey: ByteBuf::from(RECOVERY_PUBKEY_2),
        alias: "Recovery Phrase 2".to_string(),
        credential_id: None,
        purpose: Purpose::Recovery,
        key_type: KeyType::SeedPhrase,
        protection: DeviceProtection::Unprotected,
        origin: None,
    }
}

pub fn device_with_origin(origin: Option<String>) -> DeviceData {
    DeviceData {
        pubkey: ByteBuf::from(origin.as_deref().unwrap_or(PUBKEY_1)),
        alias: "My Device".to_string(),
        credential_id: Some(ByteBuf::from("credential id 1")),
        purpose: Purpose::Authentication,
        key_type: KeyType::Unknown,
        protection: DeviceProtection::Unprotected,
        origin,
    }
}

pub fn principal(device: &DeviceData) -> Principal {
    Principal::self_authenticating(&device.pubkey)
}

pub fn expect_user_error_with_message<T: std::fmt::Debug>(
    result: Result<T, CallError>,
    error_code: ErrorCode,
    message_pattern: Regex,
) {
    match result {
        Ok(_) => panic!("expected error, got {result:?}"),
        Err(CallError::Reject(_)) => panic!("expected user error, got {result:?}"),
        Err(CallError::UserError(ref user_error)) => {
            if !(user_error.code == error_code) {
                panic!(
                    "expected error code {:?}, got {:?}",
                    error_code, user_error.code
                );
            }
            if !message_pattern.is_match(&user_error.to_string()) {
                panic!("expected #{message_pattern:?}, got {user_error}");
            }
        }
    }
}

pub fn verify_security_headers(headers: &[HeaderField]) {
    let expected_headers = vec![
        ("X-Frame-Options", "DENY"),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "same-origin"),
        (
            "Permissions-Policy",
            "accelerometer=(),\
ambient-light-sensor=(),\
autoplay=(),\
battery=(),\
camera=(),\
clipboard-read=(),\
clipboard-write=(self),\
conversion-measurement=(),\
cross-origin-isolated=(),\
display-capture=(),\
document-domain=(),\
encrypted-media=(),\
execution-while-not-rendered=(),\
execution-while-out-of-viewport=(),\
focus-without-user-activation=(),\
fullscreen=(),\
gamepad=(),\
geolocation=(),\
gyroscope=(),\
hid=(),\
idle-detection=(),\
interest-cohort=(),\
keyboard-map=(),\
magnetometer=(),\
microphone=(),\
midi=(),\
navigation-override=(),\
payment=(),\
picture-in-picture=(),\
publickey-credentials-get=(self),\
screen-wake-lock=(),\
serial=(),\
speaker-selection=(),\
sync-script=(),\
sync-xhr=(self),\
trust-token-redemption=(),\
usb=(),\
vertical-scroll=(),\
web-share=(),\
window-placement=(),\
xr-spatial-tracking=()",
        ),
    ];

    for (header_name, expected_value) in expected_headers {
        let (_, value) = headers
            .iter()
            .find(|(name, _)| name.to_lowercase() == header_name.to_lowercase())
            .unwrap_or_else(|| panic!("header \"{header_name}\" not found"));
        assert_eq!(value, expected_value);
    }

    let (_, csp) = headers
        .iter()
        .find(|(name, _)| name.to_lowercase() == "content-security-policy")
        .unwrap_or_else(|| panic!("header \"Content-Security-Policy\" not found"));

    assert!(Regex::new(
        "^default-src 'none';\
connect-src 'self' https://identity.internetcomputer.org https://icp-api.io https://\\*\\.icp0.io https://\\*\\.ic0.app;\
img-src 'self' data:;\
script-src 'sha256-[a-zA-Z0-9/=+]+' 'unsafe-inline' 'unsafe-eval' 'strict-dynamic' https:;\
base-uri 'none';\
form-action 'none';\
style-src 'self' 'unsafe-inline' https://fonts\\.googleapis\\.com;\
style-src-elem 'self' 'unsafe-inline' https://fonts\\.googleapis\\.com;\
font-src https://fonts\\.gstatic\\.com;\
upgrade-insecure-requests;\
frame-ancestors 'none';$"
    )
    .unwrap()
    .is_match(csp));
}

pub fn get_metrics(env: &StateMachine, canister_id: CanisterId) -> String {
    let response = http_request(
        env,
        canister_id,
        HttpRequest {
            method: "GET".to_string(),
            url: "/metrics".to_string(),
            headers: vec![],
            body: ByteBuf::new(),
        },
    )
    .expect("HTTP request to /metrics failed");
    String::from_utf8_lossy(&response.body).to_string()
}

pub fn parse_metric(body: &str, metric: &str) -> (f64, SystemTime) {
    let metric = metric.replace('{', "\\{").replace('}', "\\}");
    let metric_capture = Regex::new(&format!("(?m)^{metric} (\\d+) (\\d+)$"))
        .unwrap()
        .captures(body)
        .unwrap_or_else(|| panic!("metric {metric} not found"));

    let metric: f64 = metric_capture.get(1).unwrap().as_str().parse().unwrap();
    let metric_timestamp = SystemTime::UNIX_EPOCH
        + Duration::from_millis(metric_capture.get(2).unwrap().as_str().parse().unwrap());
    (metric, metric_timestamp)
}

pub fn assert_metric(metrics: &str, metric_name: &str, expected: f64) {
    let (value, _) = parse_metric(metrics, metric_name);
    assert_eq!(value, expected);
}

pub fn assert_devices_equal(
    env: &StateMachine,
    canister_id: CanisterId,
    anchor: AnchorNumber,
    mut expected_devices: Vec<DeviceData>,
) {
    expected_devices.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));

    let mut devices = api::internet_identity::lookup(env, canister_id, anchor).unwrap();
    devices.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
    assert_eq!(devices, expected_devices, "expected devices to match");
}

pub fn assert_device_last_used(
    anchor_info: &IdentityAnchorInfo,
    device_key: &DeviceKey,
    expected_timestamp: u64,
) {
    let device = anchor_info
        .devices
        .iter()
        .find(|d| d.pubkey == device_key)
        .unwrap();
    assert_eq!(device.last_usage, Some(expected_timestamp));
}

pub fn time(env: &StateMachine) -> u64 {
    env.time()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

pub fn verify_delegation(user_key: UserKey, signed_delegation: &SignedDelegation, root_key: &[u8]) {
    // transform delegation into ic typed delegation so that we have access to the signature domain separator
    // (via as_signed_bytes)
    let delegation = Delegation::new(
        signed_delegation.delegation.pubkey.clone().into_vec(),
        Time::from_nanos_since_unix_epoch(signed_delegation.delegation.expiration),
    );

    // this requires imports of internal crypto infrastructure
    // -> extend state-machine-tests to offer the functionality instead (see L2-739)
    verify(
        &delegation.as_signed_bytes(),
        SignatureBytes(signed_delegation.signature.clone().into_vec()),
        public_key_bytes_from_der(user_key.as_ref()).unwrap(),
        &parse_threshold_sig_key_from_der(root_key).unwrap(),
    )
    .expect("signature invalid");
}

pub fn deploy_archive_via_ii(env: &StateMachine, ii_canister: CanisterId) -> CanisterId {
    match api::internet_identity::deploy_archive(
        env,
        ii_canister,
        ByteBuf::from(ARCHIVE_WASM.clone()),
    ) {
        Ok(DeployArchiveResult::Success(archive_principal)) => archive_principal,
        err => panic!("archive deployment failed: {err:?}"),
    }
}

pub fn install_archive_canister(env: &StateMachine, wasm: Vec<u8>) -> CanisterId {
    let canister_id = env.create_canister();
    env.install_canister(canister_id, wasm, encode_config(principal_1()));
    canister_id
}

pub fn upgrade_archive_canister(env: &StateMachine, canister_id: CanisterId, wasm: Vec<u8>) {
    env.upgrade_canister(canister_id, wasm, encode_config(principal_1()))
        .unwrap();
}

fn encode_config(authorized_principal: Principal) -> Vec<u8> {
    let config = ArchiveInit {
        ii_canister: authorized_principal,
        max_entries_per_call: 10,
        polling_interval_ns: Duration::from_secs(1).as_nanos() as u64,
        error_buffer_limit: 2,
    };
    candid::encode_one(config).expect("error encoding II installation arg as candid")
}

pub const ANCHOR_NUMBER_1: AnchorNumber = 100001;
pub const ANCHOR_NUMBER_2: AnchorNumber = 100002;
pub const ANCHOR_NUMBER_3: AnchorNumber = 100003;

pub const TIMESTAMP_1: AnchorNumber = 999991;
pub const TIMESTAMP_2: AnchorNumber = 999992;
pub const TIMESTAMP_3: AnchorNumber = 999993;

pub fn log_entry_1() -> Entry {
    Entry {
        timestamp: TIMESTAMP_1,
        anchor: ANCHOR_NUMBER_1,
        caller: principal_1(),
        operation: Operation::RegisterAnchor {
            device: DeviceDataWithoutAlias {
                pubkey: ByteBuf::from(PUBKEY_1),
                credential_id: None,
                purpose: Purpose::Authentication,
                key_type: KeyType::Unknown,
                protection: DeviceProtection::Unprotected,
                origin: None,
            },
        },
        sequence_number: 0,
    }
}

pub fn log_entry_2() -> Entry {
    Entry {
        timestamp: TIMESTAMP_2,
        anchor: ANCHOR_NUMBER_2,
        caller: principal_1(),
        operation: Operation::AddDevice {
            device: DeviceDataWithoutAlias {
                pubkey: ByteBuf::from(PUBKEY_1),
                credential_id: None,
                purpose: Purpose::Authentication,
                key_type: KeyType::Unknown,
                protection: DeviceProtection::Unprotected,
                origin: Some("foo.bar".to_string()),
            },
        },
        sequence_number: 1,
    }
}

pub fn log_entry(idx: u64, timestamp: u64, anchor: AnchorNumber) -> Entry {
    Entry {
        timestamp,
        anchor,
        caller: test_principal(idx),
        operation: Operation::UpdateDevice {
            device: ByteBuf::from(PUBKEY_1),
            new_values: DeviceDataUpdate {
                alias: None,
                credential_id: None,
                purpose: Some(Purpose::Authentication),
                key_type: None,
                protection: Some(DeviceProtection::Unprotected),
                origin: Some(Some("foo.bar".to_string())),
            },
        },
        sequence_number: idx,
    }
}

/// adapted from `PrincipalId::new_user_test_id`
pub fn test_principal(n: u64) -> Principal {
    let mut bytes = n.to_le_bytes().to_vec();
    bytes.push(0xfe); // internal marker for user test ids
    Principal::from_slice(&bytes[..])
}
