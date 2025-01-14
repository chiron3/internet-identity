// All assets
//
// This file describes which assets are used and how (content, content type and content encoding).

use crate::{http, state};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ic_cdk::api;
use lazy_static::lazy_static;
use sha2::Digest;

#[derive(Debug, PartialEq, Eq)]
pub enum ContentEncoding {
    Identity,
    GZip,
}

#[derive(Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum ContentType {
    HTML,
    JS,
    ICO,
    WEBP,
    CSS,
    OCTETSTREAM,
}

// The <script> tag that loads the 'index.js'
const JS_SETUP_SCRIPT: &str = "let s = document.createElement('script');s.async = true;s.src = 'index.js';document.head.appendChild(s);";

// Fix up HTML pages, by injecting canister ID, script tag and CSP
fn fixup_html(html: &str) -> String {
    let canister_id = api::id();
    let setup_js: String = JS_SETUP_SCRIPT.to_string();
    let html = html.replace(
        r#"<script id="setupJs"></script>"#,
        &format!(r#"<script data-canister-id="{canister_id}" id="setupJs">{setup_js}</script>"#),
    );
    html.replace(
        "<meta replaceme-with-csp/>",
        &format!(
            r#"<meta http-equiv="Content-Security-Policy" content="{}" />"#,
            &http::content_security_policy_meta()
        ),
    )
}

lazy_static! {
    // The SRI sha256 hash of the script tag, used by the CSP policy.
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Content-Security-Policy/script-src
    pub static ref JS_SETUP_SCRIPT_SRI_HASH: String = {
        let hash = &sha2::Sha256::digest(JS_SETUP_SCRIPT.as_bytes());
        let hash = BASE64.encode(hash);
        format!("sha256-{hash}")
    };

    // Various HTML pages, after the canister ID, the script tag and the CSP have been injected

    static ref INDEX_HTML_STR: String = {
        fixup_html(include_str!("../../../dist/index.html"))
    };

    static ref ABOUT_HTML_STR: String = {
        fixup_html(include_str!("../../../dist/about.html"))
    };
}

// used both in init and post_upgrade
pub fn init_assets() {
    state::assets_and_hashes_mut(|assets, asset_hashes| {
        for (path, content, content_encoding, content_type) in get_assets() {
            asset_hashes.insert(path.clone(), sha2::Sha256::digest(&content).into());
            let mut headers = match content_encoding {
                ContentEncoding::Identity => vec![],
                ContentEncoding::GZip => {
                    vec![("Content-Encoding".to_string(), "gzip".to_string())]
                }
            };
            headers.push((
                "Content-Type".to_string(),
                content_type.to_mime_type_string(),
            ));
            assets.insert(path, (headers, content));
        }
    });
}

// Get all the assets. Duplicated assets like index.html are shared and generally all assets are
// prepared only once (like injecting the canister ID).
fn get_assets() -> [(String, Vec<u8>, ContentEncoding, ContentType); 8] {
    let index_html: Vec<u8> = INDEX_HTML_STR.as_bytes().to_vec();
    let about_html: Vec<u8> = ABOUT_HTML_STR.as_bytes().to_vec();
    [
        (
            "/".to_string(),
            index_html.clone(),
            ContentEncoding::Identity,
            ContentType::HTML,
        ),
        (
            "/about".to_string(),
            about_html,
            ContentEncoding::Identity,
            ContentType::HTML,
        ),
        (
            "/index.html".to_string(),
            index_html,
            ContentEncoding::Identity,
            ContentType::HTML,
        ),
        (
            "/index.js".to_string(),
            include_bytes!("../../../dist/index.js.gz").to_vec(),
            ContentEncoding::GZip,
            ContentType::JS,
        ),
        (
            "/index.css".to_string(),
            include_bytes!("../../../dist/index.css").to_vec(),
            ContentEncoding::Identity,
            ContentType::CSS,
        ),
        (
            "/loader.webp".to_string(),
            include_bytes!("../../../dist/loader.webp").to_vec(),
            ContentEncoding::Identity,
            ContentType::WEBP,
        ),
        (
            "/favicon.ico".to_string(),
            include_bytes!("../../../dist/favicon.ico").to_vec(),
            ContentEncoding::Identity,
            ContentType::ICO,
        ),
        // Required to make II available on the identity.internetcomputer.org domain.
        // See https://github.com/r-birkner/portal/blob/rjb/custom-domains-docs-v2/docs/developer-docs/production/custom-domain/custom-domain.md#custom-domains-on-the-boundary-nodes
        (
            "/.well-known/ic-domains".to_string(),
            b"identity.internetcomputer.org".to_vec(),
            ContentEncoding::Identity,
            ContentType::OCTETSTREAM,
        ),
    ]
}
