#!/usr/bin/env bash
#
# This script creates a nice markdown table with the build artifacts' shas, links to download
# the Wasm modules, and links to the `sha256sum` steps in CI for verification.

set -euo pipefail

GITHUB_TOKEN=${INPUT_TOKEN:-${GITHUB_TOKEN:?No token given}}

PRODUCTION_ASSET=${INPUT_PRODUCTION_ASSET:?No production asset specified}
RELEASE_TAG=${RELEASE_TAG:-${GITHUB_REF_NAME:?No value for tag}}
RELEASE_TAG_PREVIOUS=${RELEASE_TAG_PREVIOUS:-}

# Starting the "intro" section where we display a short intro
section_intro=$(mktemp)
cat > "$section_intro" << EOF
This is Internet Identity release [$RELEASE_TAG](https://github.com/dfinity/internet-identity/releases/tag/$RELEASE_TAG) for commit [$GITHUB_SHA](https://github.com/dfinity/internet-identity/commit/$GITHUB_SHA).
EOF

# Starting the "build flavors" section where we add the shas of all input assets
section_build_flavors=$(mktemp)

# Start the body with a paragraph and table headers
# NOTE: throughout the doc we link to the current release (not to master) because things might
# change
cat > "$section_build_flavors" <<EOF
## Build flavors

For more information please see the [Build flavors](https://github.com/dfinity/internet-identity/tree/$RELEASE_TAG#build-features-and-flavors) section of the README.

| Filename | sha256 (links to CI Run) |
| --- | --- |
EOF

# Starting the "wasm verification" section where we explain how to build the production assets
section_wasm_verification=$(mktemp)
cat > "$section_wasm_verification" <<EOF
## Wasm Verification

To build the wasm modules yourself and verify their hashes, run the following commands from the root of the [Internet Identity repository](https://github.com/dfinity/internet-identity):
\`\`\`
git pull # to ensure you have the latest changes.
git checkout $GITHUB_SHA
./scripts/docker-build
sha256sum internet_identity.wasm.gz
./scripts/docker-build --archive
sha256sum archive.wasm
\`\`\`
EOF

# Read all "INPUT_ASSETS" (where "ASSETS" is the input specified in action.yml)
# https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#example-specifying-inputs
while IFS= read -r filename
do
    if [ -z "$filename" ]; then continue; fi
    >&2 echo working on asset "$filename"

    # Find out the Job ID
    # XXX: Unfortunately GitHub actions doesn't give us a way to find out the Job ID explicitely.
    # Instead, we find the job name that includes "$filename" without the .wasm or .wasm.gz extension and assume that's the Job ID.
    # This works because our jobs contain the filename without extension
    # (either added manually or by build matrix which takes in the filename as argument and adds it to the job name).
    # https://github.community/t/get-action-job-id/17365/7
    if [ -z "${GITHUB_RUN_ID:-}" ]
    then
        # if not running on GitHub (e.g. locally for debugging), skip
        html_url="https://example.com"
        step="step"
    else
        job_id=$(curl --silent "https://api.github.com/repos/dfinity/internet-identity/actions/runs/$GITHUB_RUN_ID/jobs" \
            | jq -cMr \
            --arg search_string "${filename%%.*}" \
            '.jobs[] | select(.name | contains($search_string)) | .id')
                    >&2 echo "Found job id: $job_id"

        # Now get the URL that we'll link to for verification of the sha
        html_url=$(curl --silent "https://api.github.com/repos/dfinity/internet-identity/actions/runs/$GITHUB_RUN_ID/jobs" \
            | jq -cMr \
            --argjson job_id "$job_id" \
            '.jobs[] | select(.id == $job_id) | .html_url')
                >&2 echo "Found html_url: $html_url"

        # Additionally grab the step number of the 'sha256sum' step
        step=$(curl --silent "https://api.github.com/repos/dfinity/internet-identity/actions/runs/$GITHUB_RUN_ID/jobs" \
            | jq -cMr \
            --argjson job_id "$job_id" \
            --arg filename "$filename" \
            '.jobs[] | select(.id == $job_id) | .steps[] | select(.name | endswith("sha256sum " + $filename)) | .number')
                >&2 echo "Found step: $step"
    fi

    # Prepare the cells:
    # | [filename.wasm](<download link>) | [<sha256>](<run link>) |
    download_link="https://github.com/dfinity/internet-identity/releases/download/$RELEASE_TAG/$filename"
    download="[\2]($download_link)"

    run_link="$html_url#step:$step:1"
    # shellcheck disable=SC2016
    sha='[`\1`]'"($run_link)"

    # Get the shasum and capture the sha (using only POSIX sed)
    shasum -a 256 "$filename"  | sed -r "s%^([a-z0-9]+)[[:space:]][[:space:]](.*)$%|$download|$sha|%" >> "$section_build_flavors"

    # Mention production asset in intro section
    if [[ "$filename" == "$PRODUCTION_ASSET" ]]
    then
        shasum -a 256 "$filename"  | sed -r "s%^([a-z0-9]+)[[:space:]][[:space:]](.*)$%The sha256 of production asset [\2]($download_link) is [\1]($run_link).%" >> "$section_intro"
    fi
done <<< "$INPUT_ASSETS"

>&2 echo "Creating release notes"

# NOTE: we create the release notes ourselves, instead of letting GitHub do it with
# 'generate_release_notes: true', here we can actually specify the release range. When doing
# it on its own, GitHub is really bad at figuring which tag to use as the previous tag (for
# listing contributions since).
# https://github.com/github/feedback/discussions/5975
section_whats_changed=$(mktemp)
if [ -z "$RELEASE_TAG_PREVIOUS" ]
then
    RELEASE_TAG_PREVIOUS=$(curl --silent \
        -H "Accept: application/vnd.github.v3+json" \
        https://api.github.com/repos/dfinity/internet-identity/releases \
        | jq -cMr 'sort_by(.published_at) | reverse | .[0] | .tag_name')
fi

>&2 echo "Using following tag as previous: $RELEASE_TAG_PREVIOUS"

jq_body=$(jq -cM -n \
    --arg previous_tag_name "$RELEASE_TAG_PREVIOUS" \
    --arg tag_name "$RELEASE_TAG" \
    '{ tag_name: $tag_name, previous_tag_name: $previous_tag_name }')

curl --silent \
    -X POST \
    -H "authorization: token $GITHUB_TOKEN" \
    -H "Accept: application/vnd.github.v3+json" \
    https://api.github.com/repos/dfinity/internet-identity/releases/generate-notes \
    --data "$jq_body" \
    | jq -cMr '.body' >> "$section_whats_changed"


body=$(mktemp)
>&2 echo "Using '$body' for body"
cat "$section_intro" >> "$body" && echo >> "$body" && rm "$section_intro"
cat "$section_whats_changed" >> "$body" && echo >> "$body" && rm "$section_whats_changed"
cat "$section_build_flavors" >> "$body" && echo >> "$body" && rm "$section_build_flavors"
cat "$section_wasm_verification" >> "$body" && echo >> "$body" && rm "$section_wasm_verification"

>&2 echo "body complete:"
cat "$body"

echo "notes-file=$body" >> "$GITHUB_OUTPUT"
