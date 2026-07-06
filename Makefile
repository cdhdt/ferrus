# Ferrus — install / uninstall (SPEC-0008 packaging).
#
#   make build            — release build of all binaries
#   sudo make install     — install binaries + polkit actions + desktop entries
#   sudo make uninstall   — remove them
#
# This is a plain install target, not distro packaging (.deb/Flatpak).
#
# Paths follow the FHS + Debian/Parrot convention (verified on the target host:
# /usr/libexec exists and is used by system helpers). CRITICAL: the helper's
# install path MUST equal the `exec.path` in the polkit actions, otherwise the
# NAMED action never matches and pkexec silently falls back to the default action.

PREFIX     ?= /usr
DESTDIR    ?=
BINDIR      = $(DESTDIR)$(PREFIX)/bin
LIBEXECDIR  = $(DESTDIR)$(PREFIX)/libexec
POLKITDIR   = $(DESTDIR)$(PREFIX)/share/polkit-1/actions
APPDIR      = $(DESTDIR)$(PREFIX)/share/applications

CARGO      ?= cargo
TARGET      = target/release
POLICY      = io.github.cdhdt.ferrus.policy

.PHONY: build install uninstall

build:
	$(CARGO) build --release

install: build
	# CLI + GUI → /usr/bin
	install -Dm755 $(TARGET)/ferrus      $(BINDIR)/ferrus
	install -Dm755 $(TARGET)/ferrus-gui  $(BINDIR)/ferrus-gui
	# Privileged helper → /usr/libexec (root-owned; MUST match polkit exec.path)
	install -Dm755 $(TARGET)/ferrus-helper $(LIBEXECDIR)/ferrus-helper
	# polkit named actions (dry-run + write, both auth_admin)
	install -Dm644 res/polkit/$(POLICY)  $(POLKITDIR)/$(POLICY)
	# desktop entries: GPU default + a software-rendering variant
	install -Dm644 res/applications/io.github.cdhdt.ferrus.desktop \
		$(APPDIR)/io.github.cdhdt.ferrus.desktop
	install -Dm644 res/applications/io.github.cdhdt.ferrus-software.desktop \
		$(APPDIR)/io.github.cdhdt.ferrus-software.desktop
	@echo
	@echo "Installed. The GUI now elevates via the NAMED polkit action"
	@echo "(exec.path = $(PREFIX)/libexec/ferrus-helper). Launch: ferrus-gui"

uninstall:
	rm -f $(BINDIR)/ferrus $(BINDIR)/ferrus-gui
	rm -f $(LIBEXECDIR)/ferrus-helper
	rm -f $(POLKITDIR)/$(POLICY)
	rm -f $(APPDIR)/io.github.cdhdt.ferrus.desktop
	rm -f $(APPDIR)/io.github.cdhdt.ferrus-software.desktop
	@echo "Uninstalled."
