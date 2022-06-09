/** The functions here are derived (manually) from Internet Identity's Candid file */

use crate::framework;
use ic_state_machine_tests::{CanisterId, StateMachine};
use internet_identity_interface as types;

/// A fake "health check" method that just checks the canister is alive a well.
pub fn health_check(env: &StateMachine, canister_id: CanisterId) {
    let user_number: types::UserNumber = 0;
    let _: (Vec<types::DeviceData>,) = framework::call_candid(env, canister_id, "lookup", (user_number,)).unwrap();
}

pub fn create_challenge(env: &StateMachine, canister_id: CanisterId) -> types::Challenge {
    let (c,) = framework::call_candid(env, canister_id, "create_challenge", ()).unwrap();
    c
}

pub fn register(env: &StateMachine, canister_id: CanisterId, device_data: types::DeviceData, challenge_attempt: types::ChallengeAttempt) -> types::RegisterResponse {
    match framework::call_candid_as(env, canister_id, framework::some_principal(), "register", (device_data, challenge_attempt)) {
        Ok((r,)) => r,
        Err(e) => panic!("Failed to register: {:?}", e),
    }
}

pub fn lookup(env: &StateMachine, canister_id: CanisterId, user_number: types::UserNumber) -> Vec<types::DeviceData> {
    match framework::call_candid_as(env, canister_id, framework::some_principal(), "lookup", (user_number,)) {
        Ok((r,)) => r,
        Err(e) => panic!("Failed to lookup: {:?}", e),
    }
}