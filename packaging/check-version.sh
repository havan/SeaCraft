#!/usr/bin/env bash
# Print each program's version, and fail when a tag build is not exactly that release.
# Usage: check-version.sh VERSION PROGRAM...
#
# A build from the clean tag vVERSION says `NAME VERSION`. Anything else names its commit, such as
# `0.3.0+67-ga4b51c2-dirty` or `0.3.1-dev+gabc1234`, which a release must never ship with.
set -euo pipefail

version=$1
shift
for program in "$@"; do
  said=$("$program" --version)
  echo "$said"
  case $said in
    *" $version") ;;
    *" $version+"* | *" $version-dev+"* | *" $version.dev+"*)
      if [ "${GITHUB_REF_TYPE:-}" = tag ]; then
        echo "a release build must say $version alone, not ${said##* }" >&2
        exit 1
      fi
      ;;
    *)
      echo "$program does not say version $version" >&2
      exit 1
      ;;
  esac
done
