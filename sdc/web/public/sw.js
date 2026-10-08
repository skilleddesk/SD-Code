// SDC Anywhere service worker: shows a push and opens the link in it. That is all it does.
//
// - No `fetch` handler and no cache: the page always comes straight from the relay (pinning the page's code is a later phase).
// - The push carries only a kind and a link ({ t: 'approval', url: '/a/<id>' }); the details of a request are never in it.
// - Whatever arrives is treated as untrusted: a message that is not exactly that shape gets the plain wording and the home page,
//   and a link is only ever opened on this site.

'use strict';

var WORDS = {
  en: { title: 'SDC needs you', body: 'Something on your computer is waiting for your answer.' },
  bn: { title: 'SDC-র আপনাকে দরকার', body: 'আপনার কম্পিউটারে কিছু একটা আপনার উত্তরের অপেক্ষায় আছে।' },
};

var APPROVAL_URL = /^\/a\/[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$/;

function wordsFor(languages) {
  for (var i = 0; i < languages.length; i++) {
    var base = String(languages[i] || '').toLowerCase().split('-')[0];

    if (WORDS[base]) return WORDS[base];
  }

  return WORDS.en;
}

/** What to show for a push payload. Anything unexpected becomes the plain notification pointing at the home page. */
function describe(data, languages) {
  var words = wordsFor(languages || []);
  var url = '/';
  var tag = 'sdc';

  try {
    var message = data ? data.json() : null;

    if (message && message.t === 'approval' && typeof message.url === 'string' && APPROVAL_URL.test(message.url)) {
      url = message.url;
      tag = 'sdc' + message.url.slice(2);
    }
  } catch (error) {
    // Not JSON: show the plain notification.
  }

  return { title: words.title, options: { body: words.body, icon: '/icon-192.png', badge: '/icon-192.png', tag: tag, data: { url: url } } };
}

/** A link from a notification, made safe: same site only, as a path. */
function safeTarget(url, origin) {
  // Only a path on this site: a string that starts with one slash (not two, which would name another host).
  if (typeof url !== 'string' || url.charAt(0) !== '/' || url.charAt(1) === '/' || url.charAt(1) === '\\') return '/';

  try {
    var target = new URL(url, origin);

    return target.origin === origin ? target.pathname : '/';
  } catch (error) {
    return '/';
  }
}

self.addEventListener('install', function () {
  self.skipWaiting();
});

self.addEventListener('activate', function (event) {
  event.waitUntil(self.clients.claim());
});

self.addEventListener('push', function (event) {
  // A browser requires every push to show something (`userVisibleOnly`), so this always does.
  var shown = describe(event.data, self.navigator && self.navigator.languages);

  event.waitUntil(self.registration.showNotification(shown.title, shown.options));
});

self.addEventListener('notificationclick', function (event) {
  event.notification.close();

  var path = safeTarget(event.notification.data && event.notification.data.url, self.location.origin);

  event.waitUntil(
    self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then(function (windows) {
      for (var i = 0; i < windows.length; i++) {
        if (new URL(windows[i].url).origin === self.location.origin && 'focus' in windows[i]) {
          // An open page is told where to go and brought forward; it needs no reload.
          windows[i].postMessage({ t: 'open', url: path });

          return windows[i].focus();
        }
      }

      return self.clients.openWindow(path);
    }),
  );
});
