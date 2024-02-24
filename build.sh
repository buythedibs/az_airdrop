#!/usr/bin/env bash

set -eu

cargo +stable contract build --release
