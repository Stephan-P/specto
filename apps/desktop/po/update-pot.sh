#!/bin/sh
# Collects the desktop app's translatable strings into po/specto.pot, the
# template translations start from. Run it after changing strings.
#
#     apps/desktop/po/update-pot.sh
#
# Needs xgettext from GNU gettext 0.24 or newer (the first that reads Rust).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/.."
# The functions of src/i18n.rs. Plain file order, so the template doesn't
# change with the locale of whoever runs this.
xgettext --language=Rust --from-code=UTF-8 --add-comments=Translators: --no-git \
    --keyword=gettext --keyword=gettext_f \
    --keyword=ngettext_f:1,2 \
    --package-name=specto --msgid-bugs-address=https://github.com/xfbs/specto/issues \
    --copyright-holder="The Specto contributors" \
    --output=po/specto.pot \
    $(LC_ALL=C ls src/*.rs)
# Without the creation date, running this again changes nothing unless a
# string did.
sed -i.bak '/^"POT-Creation-Date: /d' po/specto.pot && rm po/specto.pot.bak
echo "$here/specto.pot: $(grep -c '^msgid ' po/specto.pot) strings"
