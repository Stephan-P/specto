#!/usr/bin/env bash
# System packages needed to build pdfrs on Fedora. Shared by GitHub Actions and
# (later) GitLab CI so both run the same environment.
set -euo pipefail
dnf install -y --setopt=install_weak_deps=False \
  gcc pkgconf-pkg-config git curl \
  gtk4-devel libadwaita-devel \
  poppler-utils qpdf
