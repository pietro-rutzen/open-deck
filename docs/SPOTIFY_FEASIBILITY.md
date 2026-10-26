# Spotify feasibility — 2026-10-25

Spotify supports DJ mixing in selected partner apps: [Spotify Support](https://support.spotify.com/in-en/article/dj-integration/). OpenDeck has no such partner access.

Public [Developer Policy](https://developer.spotify.com/policy/) III.7 prohibits segueing, mixing, remixing or overlapping Spotify content with other audio; [compliance tips](https://developer.spotify.com/compliance-tips) explicitly list DJ/mixing apps. Web Playback SDK is a protected player, not a PCM feed for the Rust mixer.

Full Spotify mixing is blocked pending authorized partner access. OAuth alone cannot solve this. No audio extraction or DRM bypass is included. Proceed with local DRM-free tracks. Any future control-only integration needs separate scoping and policy verification. AI remains excluded from the product.
