# সাইনড রিলিজ ও অটো-আপডেট - মালিকের যা যা জোগাড় করতে হবে (Phase 4.1)

বর্তমান অবস্থা: রিলিজ বিল্ড **সাইন ছাড়া** তৈরি হয়। নিচের জিনিসগুলো GitHub-এ `Settings → Secrets and variables → Actions`-এ যোগ করলে `release.yml` নিজে থেকে সেই ধাপগুলো চালু করে। কোনো সিক্রেট না থাকলে আগের মতোই (সাইন ছাড়া) বিল্ড হয়, ভাঙে না।

| কী চান | কোথা থেকে | GitHub Secret-এর নাম | খরচ |
|---|---|---|---|
| **macOS সাইন ও নোটারাইজ** (Gatekeeper-এর সতর্কবার্তা থাকবে না) | Apple Developer Program (developer.apple.com) → "Developer ID Application" সার্টিফিকেট (.p12 ফাইল, base64 করে) | `APPLE_CERTIFICATE` (base64 .p12), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID` | বছরে প্রায় ১০০ ডলার |
| **Windows সাইন** (SmartScreen-এর সতর্কবার্তা কমে) | একটি Code Signing সার্টিফিকেট (.pfx) - কোনো CA থেকে (Sectigo, DigiCert ইত্যাদি) | `WINDOWS_CERTIFICATE` (base64 .pfx), `WINDOWS_CERTIFICATE_PASSWORD` | বছরে প্রায় ১০০-৪০০ ডলার |
| **সাইনড অটো-আপডেট** (অ্যাপ নিজে নতুন সংস্করণ নামায়) | কোনো দোকান লাগে না। নিজের কম্পিউটারে একটি কী-জোড়া বানান: `pnpm --filter @sdc/app tauri signer generate -w ~/.tauri/sdc.key` | `TAURI_SIGNING_PRIVATE_KEY` (প্রাইভেট কী-র লেখা), `TAURI_UPDATER_PUBKEY` (পাবলিক কী) | বিনামূল্যে |

## অটো-আপডেট চালু করার ধাপ (বিনামূল্যে)
1. ওপরের কমান্ডে কী বানান। প্রাইভেট কী কোথাও কমিট/শেয়ার করবেন না; চ্যাটেও পেস্ট করবেন না।
2. প্রাইভেট কী-র লেখা `TAURI_SIGNING_PRIVATE_KEY` নামে এবং পাবলিক কী `TAURI_UPDATER_PUBKEY` নামে GitHub Secret-এ বসান।
3. নতুন ট্যাগ (যেমন `v3.1.1`) পুশ করুন। বিল্ড `latest.json` ও সাইনড আপডেট ফাইল রিলিজে যোগ করবে এবং অ্যাপ সেখান থেকে আপডেট পাবে।
4. সংস্করণ ৩.১.০-এর অ্যাপ (আপডেটার ছাড়া বিল্ড) নিজে আপডেট পাবে না - প্রথমবার হাতে ইনস্টল করতে হবে।

## সততার সাথে যা এখনো প্রমাণ হয়নি
- এই ধাপগুলো CI-তে সিক্রেট ছাড়া চালানো যায়নি, তাই `release.yml`-এর নতুন ধাপের YAML বৈধ (পার্স হয়েছে) কিন্তু সিক্রেটসহ চলার পরীক্ষা **হয়নি**।
- অ্যাপে আপডেট বোতাম/ডায়ালগ এখনো যোগ করা হয়নি; প্লাগইন শুধু বিল্ডে যুক্ত হয়। (বাকি কাজ।)
