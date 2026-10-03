#!/usr/bin/env bash
set -eu

# The session supplies a fresh HOME. Seed only browser preferences, never a
# user's existing profile or credentials.
profile="$HOME/.matinee-google-firefox"
mkdir -p "$profile"
cat > "$profile/user.js" <<'EOF'
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("browser.rights.3.shown", true);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("datareporting.policy.dataSubmissionPolicyBypassNotification", true);
EOF

export MOZ_ACCESSIBILITY_ATK2=1
exec firefox --kiosk -profile "$profile" 'https://www.google.com/'
