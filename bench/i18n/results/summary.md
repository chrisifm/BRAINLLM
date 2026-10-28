# QA #153 — i18n results

brain `018520473d6e` · 2026-10-14T00:44:50.898Z · PASS 80 · FAIL 0 · BLOCKED 0 · INFO 2

| group | check | status | evidence | owner |
|---|---|---|---|---|
| 1 app | check | PASS | brain check --json . → [] |  |
| 1 app | doctor | PASS | brain doctor → exit 0 · SEO 82–95 over 9 pages |  |
| 1 app | probe: t() in <meta seo> | PASS | es <title> "A site in three languages [es] · Polyglot probe" |  |
| 2 browser | localized <title> per locale | PASS | via t() in <meta seo> |  |
| 1 app | probe: <LangSwitch> builds | PASS | layout with <LangSwitch /> builds |  |
| 1 app | probe: t() in app.brain shell | PASS | es shell shows "(absent)" |  |
| 1 app | info: in-locale links | INFO | href="/about" on /es stays "/about" (switches the visitor to English); no localized link helper exists, so the example keeps per-locale paths in nav.*_href keys | RUNTIME/BUILD |
| 1 app | spec teaches i18n | PASS | brain spec documents brain.json i18n |  |
| 1 app | brain add locale | PASS | exit 0, brain.json locales ["en","es","ar","fr"], i18n/fr.json 26/26 keys |  |
| 1 app | brain i18n --missing | PASS | exit 0: {"default":"en","locales":{"es":{"missing":[],"untranslated":[]},"ar":{"missing":[],"untranslated":[]},"fr":{"missing":[],"untranslated":["about.body","about.ti |  |
| 2 browser | url en home | PASS | / → 200, location / |  |
| 2 browser | html lang/dir en home | PASS | <html lang="en" dir="ltr" data-default-lang="en"> |  |
| 2 browser | texts en home | PASS | h1 "A site in three languages" |  |
| 2 browser | hreflang en home | PASS | ar=/ar en=/ es=/es x-default=/ |  |
| 2 browser | layout direction en home | PASS | brand x=24 nav x=253 direction ltr (brand left of nav) |  |
| 3 network | network en home | PASS | 8 requests · dictionaries none (static page) |  |
| 2 browser | url en about | PASS | /about → 200, location /about |  |
| 2 browser | html lang/dir en about | PASS | <html lang="en" dir="ltr" data-default-lang="en"> |  |
| 2 browser | texts en about | PASS | h1 "About this probe" |  |
| 2 browser | hreflang en about | PASS | ar=/ar/about-us en=/about es=/es/about-us x-default=/about |  |
| 2 browser | layout direction en about | PASS | brand x=24 nav x=253 direction ltr (brand left of nav) |  |
| 3 network | network en about | PASS | 8 requests · dictionaries none (static page) |  |
| 2 browser | url en counter | PASS | /counter → 200, location /counter |  |
| 2 browser | html lang/dir en counter | PASS | <html lang="en" dir="ltr" data-default-lang="en"> |  |
| 2 browser | texts en counter | PASS | h1 "Click counter" |  |
| 2 browser | hreflang en counter | PASS | ar=/ar/counter en=/counter es=/es/counter x-default=/counter |  |
| 2 browser | layout direction en counter | PASS | brand x=24 nav x=253 direction ltr (brand left of nav) |  |
| 2 browser | plural en counter | PASS | "1 click so far" → click → "2 clicks so far" |  |
| 2 browser | placeholder en counter | PASS | "Hello Ada, welcome back" |  |
| 2 browser | fmt en counter | PASS | Mar 14, 2026 · 1,234,567.5 · $1,999.50 |  |
| 2 browser | fmt prerender = browser en counter | PASS | identical |  |
| 3 network | network en counter | PASS | 13 requests · dictionaries i18n-counter-en.xsbfkue1.js (648 B) |  |
| 2 browser | url es home | PASS | /es → 200, location /es |  |
| 2 browser | html lang/dir es home | PASS | <html lang="es" dir="ltr" data-default-lang="en"> |  |
| 2 browser | texts es home | PASS | h1 "A site in three languages [es]" |  |
| 2 browser | hreflang es home | PASS | ar=/ar en=/ es=/es x-default=/ |  |
| 2 browser | layout direction es home | PASS | brand x=24 nav x=222 direction ltr (brand left of nav) |  |
| 3 network | network es home | PASS | 8 requests · dictionaries none (static page) |  |
| 2 browser | url es about | PASS | /es/about-us → 200, location /es/about-us |  |
| 2 browser | html lang/dir es about | PASS | <html lang="es" dir="ltr" data-default-lang="en"> |  |
| 2 browser | texts es about | PASS | h1 "About this probe [es]" |  |
| 2 browser | hreflang es about | PASS | ar=/ar/about-us en=/about es=/es/about-us x-default=/about |  |
| 2 browser | layout direction es about | PASS | brand x=24 nav x=222 direction ltr (brand left of nav) |  |
| 3 network | network es about | PASS | 8 requests · dictionaries none (static page) |  |
| 2 browser | url es counter | PASS | /es/counter → 200, location /es/counter |  |
| 2 browser | html lang/dir es counter | PASS | <html lang="es" dir="ltr" data-default-lang="en"> |  |
| 2 browser | texts es counter | PASS | h1 "Click counter [es]" |  |
| 2 browser | hreflang es counter | PASS | ar=/ar/counter en=/counter es=/es/counter x-default=/counter |  |
| 2 browser | layout direction es counter | PASS | brand x=24 nav x=222 direction ltr (brand left of nav) |  |
| 2 browser | plural es counter | PASS | "1 click so far [es]" → click → "2 clicks so far [es]" |  |
| 2 browser | placeholder es counter | PASS | "Hello Ada, welcome back [es]" |  |
| 2 browser | fmt es counter | PASS | 14 mar 2026 · 1.234.567,5 · 1999,50 US$ |  |
| 2 browser | fmt prerender = browser es counter | PASS | identical |  |
| 3 network | network es counter | PASS | 13 requests · dictionaries i18n-counter-es.leiseelf.js (663 B) |  |
| 2 browser | url ar home | PASS | /ar → 200, location /ar |  |
| 2 browser | html lang/dir ar home | PASS | <html lang="ar" dir="rtl" data-default-lang="en"> |  |
| 2 browser | texts ar home | PASS | h1 "A site in three languages [ar]" |  |
| 2 browser | hreflang ar home | PASS | ar=/ar en=/ es=/es x-default=/ |  |
| 2 browser | layout direction ar home | PASS | brand x=585 nav x=242 direction rtl (brand must sit right of nav) |  |
| 3 network | network ar home | PASS | 8 requests · dictionaries none (static page) |  |
| 2 browser | url ar about | PASS | /ar/about-us → 200, location /ar/about-us |  |
| 2 browser | html lang/dir ar about | PASS | <html lang="ar" dir="rtl" data-default-lang="en"> |  |
| 2 browser | texts ar about | PASS | h1 "About this probe [ar]" |  |
| 2 browser | hreflang ar about | PASS | ar=/ar/about-us en=/about es=/es/about-us x-default=/about |  |
| 2 browser | layout direction ar about | PASS | brand x=585 nav x=242 direction rtl (brand must sit right of nav) |  |
| 3 network | network ar about | PASS | 8 requests · dictionaries none (static page) |  |
| 2 browser | url ar counter | PASS | /ar/counter → 200, location /ar/counter |  |
| 2 browser | html lang/dir ar counter | PASS | <html lang="ar" dir="rtl" data-default-lang="en"> |  |
| 2 browser | texts ar counter | PASS | h1 "Click counter [ar]" |  |
| 2 browser | hreflang ar counter | PASS | ar=/ar/counter en=/counter es=/es/counter x-default=/counter |  |
| 2 browser | layout direction ar counter | PASS | brand x=585 nav x=242 direction rtl (brand must sit right of nav) |  |
| 2 browser | plural ar counter | PASS | "1 click so far [ar]" → click → "2 clicks so far [ar]" |  |
| 2 browser | placeholder ar counter | PASS | "Hello Ada, welcome back [ar]" |  |
| 2 browser | fmt ar counter | PASS | 14‏/03‏/2026 · 1,234,567.5 · ‏1,999.50 US$ |  |
| 2 browser | fmt prerender = browser ar counter | PASS | identical |  |
| 3 network | network ar counter | PASS | 13 requests · dictionaries i18n-counter-ar.e7f53ft6.js (662 B) |  |
| 3 network | switch: nothing before | PASS | open /counter: i18n-counter-en.xsbfkue1.js of 13 requests |  |
| 3 network | switch en → es (link) | PASS | → /es/counter lang=es, "1 click so far [es]"; es dictionary fetched on click (i18n-counter-es.leiseelf.js) |  |
| 3 network | info: hover prerender | INFO | hovering the es link (speculation rules, eagerness moderate) already downloads 1 requests: /es/counter | BUILD |
| 3 network | switch es → ar (dir flips) | PASS | → /ar/counter lang=ar dir=rtl, brand x=585 nav x=242, ar dictionary loaded then |  |
| 4 llm | llm claude-sonnet-5-5 | PASS | i18n {"default":"en","locales":["en","es"]} · files en+es · check [] · doctor 0 · views 5/2 · switch yes · add locale not called · seo.minScore 80 · 42 calls · 45 s |  |
| 4 llm | llm claude-haiku-5-5 | PASS | i18n {"default":"en","locales":["en","es"]} · files en+es · check [] · doctor 0 · views 6/2 · switch yes · add locale not called · seo.minScore 80 · 98 calls · 155 s |  |

## Dictionary chunks

| chunk | bytes | gzip | keys |
|---|---:|---:|---:|
| i18n-404-en.se3wmsjn.js | 389 | 240 | 11 |
| i18n-about-ar.otxse7ce.js | 419 | 250 | 10 |
| i18n-about-en.18693xbm.js | 373 | 239 | 10 |
| i18n-about-es.fwl461y1.js | 419 | 250 | 10 |
| i18n-counter-ar.e7f53ft6.js | 621 | 311 | 15 |
| i18n-counter-en.xsbfkue1.js | 545 | 297 | 15 |
| i18n-counter-es.leiseelf.js | 621 | 312 | 15 |
| i18n-home-ar.km2o8swk.js | 450 | 256 | 11 |
| i18n-home-en.wzgtirb3.js | 399 | 244 | 11 |
| i18n-home-es.qk0zxwtz.js | 450 | 257 | 11 |
| i18n-offline-ar.g7d31f81.js | 431 | 252 | 11 |
| i18n-offline-en.yjuxhj9h.js | 380 | 239 | 11 |
| i18n-offline-es.uxklot9e.js | 431 | 252 | 11 |
