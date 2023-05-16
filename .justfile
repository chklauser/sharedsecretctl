#!/usr/bin/env just --justfile

set windows-shell := ["C:/Program Files/PowerShell/7/pwsh.exe", "-c"]

help:
    @just --list

# Re-generate the CRD (tooling-crd.yml). This is intended for tooling only.
# Deployments should always use the freshly generated CRD yaml.
tooling-crd:
    cargo run --bin crdgen > tooling-crd.yml
