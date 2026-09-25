#!/bin/sh
# Versuch, tgcalls auf das heutige WebRTC zu heben, statt Telegrams Fork
# tg_owt mitzubauen.
#
# Upstream hat rtc:: und cricket:: in webrtc:: zusammengelegt und sigslot
# entfernt. Das ist zum groessten Teil eine Umbenennung; was danach noch
# uebrig bleibt, sind echte Schnittstellenaenderungen -- und genau die
# wollen wir zaehlen, bevor wir uns fuer einen Weg entscheiden.
set -e
B=/run/media/sebastian/2e638a7f-26db-4e89-9446-81d688464798/webrtc-bau
rm -rf $B/tgcalls-neu $B/tgcalls-schicht
cp -a $B/tgcalls $B/tgcalls-neu

find $B/tgcalls-neu -name "*.h" -o -name "*.cpp" -o -name "*.mm" | while read f; do
    sed -i -e 's/\brtc::/webrtc::/g' \
           -e 's/\bcricket::/webrtc::/g' \
           -e 's/^namespace rtc {/namespace webrtc {/' \
           -e 's/^namespace cricket {/namespace webrtc {/' \
           -e 's|// namespace rtc|// namespace webrtc|' \
           -e 's|// namespace cricket|// namespace webrtc|' "$f"
done

# sigslot ist nicht umbenannt, sondern geloescht worden -- also mitgeben.
mkdir -p $B/tgcalls-schicht/rtc_base/third_party/sigslot $B/tgcalls-schicht/api/transport
cp $B/tg_owt/src/rtc_base/third_party/sigslot/sigslot.h $B/tgcalls-schicht/rtc_base/third_party/sigslot/
cp $B/tg_owt/src/rtc_base/third_party/sigslot/sigslot.cc $B/tgcalls-schicht/rtc_base/third_party/sigslot/
cp $B/tg_owt/src/api/transport/field_trial_based_config.h $B/tgcalls-schicht/api/transport/
echo umbenannt
