#!/usr/bin/env python3

import subprocess
import sys
import toml


def get_previous_version():
    subprocess.run(['git', 'fetch', 'origin', 'main'])
    previous_version = subprocess.check_output(
        ['git', 'show', 'origin/main:./mm2src/mm2_bin_lib/Cargo.toml']).decode()
    cargo_data = toml.loads(previous_version)
    previous_version = cargo_data['package']['version']

    return previous_version


def get_current_version():
    with open('./mm2src/mm2_bin_lib/Cargo.toml', 'r') as file:
        cargo_toml_pr = file.read()

    cargo_data_pr = toml.loads(cargo_toml_pr)

    return cargo_data_pr['package']['version']


current_version = get_current_version()
previous_version = get_previous_version()

print(f"Main branch mm2 version: {previous_version}")
print(f"Current branch mm2 version: {current_version}")

if previous_version == current_version:
    print("Bump the mm2 version before merge!")
    sys.exit(1)
