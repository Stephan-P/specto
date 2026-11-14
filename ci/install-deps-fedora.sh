#!/usr/bin/env bash
# System packages needed to build pdfrs on Fedora. Shared by GitHub Actions and
# (later) GitLab CI so both run the same environment.
set -euo pipefail
dnf install -y --setopt=install_weak_deps=False \
  gcc pkgconf-pkg-config git curl \
  gtk4-devel libadwaita-devel \
  poppler-utils qpdf \
  mutter dbus-daemon glib2 python3 python3-gobject mesa-dri-drivers
# The last line is for the headless desktop app tests (tools/headless):
# a compositor, a session bus, gdbus, and software OpenGL.
