#!/bin/bash
# Demo data for the callboard storyboard, set up with callboard's own CLI
# before the app starts. The service starts on the first command.
set -euo pipefail
cd "$(dirname "$0")"

callboard put reviews < fixtures/reviews.json > /dev/null
callboard put nightly < fixtures/nightly.json > /dev/null
callboard put releases < fixtures/releases.json > /dev/null

inbox=$(callboard board add Inbox | jq -r .id)
release=$(callboard board add 'Release 0.2' | jq -r .id)
callboard board color "$release" blue > /dev/null
for title in "Reply to Sam about the session-store race" "Book the room for Thursday's retro"; do
  callboard todo add "$inbox" "$title" > /dev/null
done
callboard note add "$inbox" 'Release freeze starts Thursday at 17:00.' --title Freeze > /dev/null
for title in "Write the upgrade notes" "Tag the release candidate"; do
  callboard todo add "$release" "$title" > /dev/null
done

# A watch: support tickets the user follows, reported on by a script. Its
# first report is the baseline, so nothing needs attention yet.
title="Tickets I'm waiting on"
callboard watch report tickets --title "$title" < /dev/null > /dev/null
callboard watch item add tickets https://support.example.com/t/1182 --label 'My refund' > /dev/null
callboard watch item add tickets https://support.example.com/t/1190 --label 'Safari login bug' > /dev/null
callboard watch item add tickets https://support.example.com/t/1201 --label 'New invoice address' > /dev/null
callboard watch report tickets --title "$title" < fixtures/tickets.json > /dev/null

# The layout the app opens: feeds on the left, boards on the right, and the
# watch past the right edge of the view, found by zooming out.
card() { printf '{"target":%s,"x":%s,"y":%s,"width":%s,"height":%s,"collapsed":false}' "$@"; }
printf '{"view":{"x":0,"y":0},"cards":[%s,%s,%s,%s,%s]}' \
  "$(card '{"kind":"watch","name":"tickets"}' 940 12 420 300)" \
  "$(card '{"kind":"feed","name":"reviews"}' 12 12 450 330)" \
  "$(card '{"kind":"feed","name":"nightly"}' 12 354 450 236)" \
  "$(card "{\"kind\":\"board\",\"id\":$release}" 474 400 440 250)" \
  "$(card "{\"kind\":\"board\",\"id\":$inbox}" 474 12 440 376)" \
  | callboard layout save Day > /dev/null
# The CLI has no command for the app's starting layout; set it through
# the service's API, as the app does.
curl -sf --unix-socket "$XDG_RUNTIME_DIR/callboard.sock" -X PATCH \
  -H 'content-type: application/json' -d '{"last_layout":"Day"}' \
  http://callboard/preferences > /dev/null
