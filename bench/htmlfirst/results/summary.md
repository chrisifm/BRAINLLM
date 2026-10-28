# HTML first QA (#157) — 2026-10-13

Binary sha256 `2b83cfa6ec91` (~/tmp/brainllm-htmlfirst/bin/brain), Node v26.8.1. 89/89 checks pass, 0 fail.


| App | Page / form | Check | Mode | Result | Evidence | Owner |
|---|---|---|---|---|---|---|
| scaffold | build | `brain build` succeeds | build | PASS | exit 0; summary 13 pages · 218.2 KB gz total · SEO 86–95 · 133 written, 0 unchanged · 243 ms |  |
| examples | build | `brain build` succeeds as shipped | build | PASS | exit 0; ok |  |
| scaffold | brain start | serves dist + server.mjs (/_health 200) | start | PASS | port 58056 |  |
| examples | brain start | serves dist + server.mjs (/_health 200) | start | PASS | port 58059 |  |
| scaffold | / | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /; h1 "Your BRAINLLM app is alive."; main text 312 chars |  |
| scaffold | / | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /about | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /about; h1 "About scaf"; main text 492 chars |  |
| scaffold | /about | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /posts | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /posts; h1 "Posts"; main text 217 chars |  |
| scaffold | /posts | every internal link loads its page (7) | no-JS | PASS | / /about /posts /account /login /posts/hello-world /posts/second-post |  |
| scaffold | /posts/hello-world | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /posts/hello-world; h1 "Hello, world"; main text 192 chars |  |
| scaffold | /posts/hello-world | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /posts/second-post | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /posts/second-post; h1 "A second post"; main text 124 chars |  |
| scaffold | /posts/second-post | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /login | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /login; h1 "Sign in"; main text 30 chars |  |
| scaffold | /login | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /account | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 303→200 url /login?next=/account; h1 "Sign in"; main text 30 chars |  |
| scaffold | /account | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /403 | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /403; h1 "Access denied"; main text 69 chars |  |
| scaffold | /403 | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /no-such-page | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 404 url /no-such-page; h1 "Page not found"; main text 99 chars |  |
| scaffold | /no-such-page | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /probe-form | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /probe-form; h1 "Probe form"; main text 82 chars |  |
| scaffold | /probe-form | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /probe-client | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /probe-client; h1 "Probe client"; main text 88 chars |  |
| scaffold | /probe-client | every internal link loads its page (5) | no-JS | PASS | / /about /posts /account /login |  |
| scaffold | /login | form() with a `server:` submit is a real <form method="post"> (name= on fields, hidden _brain) | no-JS | PASS | method post; _brain "xq8y/login"; names email,password,_brain |  |
| scaffold | /probe-form | form() with a `server:` submit is a real <form method="post"> (name= on fields, hidden _brain) | no-JS | PASS | method post; _brain "vx8w/send"; names name,email,age,_brain |  |
| scaffold | /login | wrong credentials → 422, same page, typed email kept, "Wrong email or password" shown | no-JS | PASS | status 422 url /login?next=/posts; values {"email":"qa@example.com"}; messages ["Wrong email or password"]; POSTs 1 |  |
| scaffold | /login | invalid input → 422, value kept, form() messages shown | no-JS | PASS | status 422; values {"email":"not-an-email"}; messages ["Enter a valid email","Required"] |  |
| scaffold | /login | right credentials → 303 to the `next` page (/posts) | no-JS | PASS | docs 303 → /posts, 200; final url /posts; POSTs 1 |  |
| scaffold | /account | after the no-JS login the session works (account shows the user) | no-JS | PASS | url /account; "Your account\n\nSigned in as qa@example.com.\n\nSign out" |  |
| scaffold | /probe-form | invalid → 422, same page, typed values kept, form() messages | no-JS | PASS | status 422 url /probe-form; values {"name":"","email":"bad","age":"12"}; messages ["Required","Enter a valid email","Must be at least 18"]; POSTs 1 |  |
| scaffold | /probe-form | server { errors } → shown on its field | no-JS | PASS | status 422 url /probe-form; values {"name":"taken","email":"qa@example.com","age":"20"}; messages ["That name is taken"]; POSTs 1 |  |
| scaffold | /probe-form | unexpected server failure → generic message | no-JS | PASS | status 422 url /probe-form; values {"name":"boom","email":"qa@example.com","age":"20"}; messages ["Something went wrong. Try again."]; POSTs 1 |  |
| scaffold | /probe-form | valid, function returns { redirect: "/about" } → 303 to /about | no-JS | PASS | status 303→200 url /about; values {}; messages []; POSTs 1 |  |
| scaffold | /probe-form | valid → 303 PRG to the same page | no-JS | PASS | status 303→200 url /probe-form; values {"name":"","email":"","age":""}; messages []; POSTs 1 |  |
| scaffold | /probe-client | <meta client> renders a reserved-size placeholder | no-JS | PASS | placeholder height 320px (meta client height="320"); #after top 573px |  |
| scaffold | POST /probe-form | cross-site POST (Origin: https://evil.example) → 403 | raw HTTP | PASS | status 403; body "Cross-origin form post" |  |
| scaffold | POST /probe-form | cross-site POST (Sec-Fetch-Site: cross-site, no Origin) → 403 | raw HTTP | PASS | status 403; body "Cross-origin form post" |  |
| scaffold | POST /probe-form | missing Origin and Sec-Fetch-Site → 403 | raw HTTP | PASS | status 403; body "Cross-origin form post" |  |
| scaffold | POST /probe-form | JSON body → 415 | raw HTTP | PASS | status 415; body "Forms are sent as application/x-www-form-urlencoded" |  |
| scaffold | POST /probe-form | unknown _brain (unknown hash) → 400 | raw HTTP | PASS | status 400; body "Unknown form" |  |
| scaffold | POST /probe-form | unknown _brain (known hash, unknown function) → 400 | raw HTTP | PASS | status 400; body "Unknown form" |  |
| scaffold | POST /probe-form | missing _brain → 400 | raw HTTP | PASS | status 400; body "Unknown form" |  |
| scaffold | POST /probe-form | control: same origin (Origin) → 303 | raw HTTP | PASS | status 303 location /probe-form; body "" |  |
| scaffold | POST /probe-form | control: same origin (Sec-Fetch-Site only) → 303 | raw HTTP | PASS | status 303 location /probe-form; body "" |  |
| scaffold | / | renders and hydrates | JS | PASS | url /; hydrated true; h1 "Your BRAINLLM app is alive." |  |
| scaffold | / | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | /about /posts /account /login |  |
| scaffold | /about | renders and hydrates | JS | PASS | url /about; hydrated true; h1 "About scaf" |  |
| scaffold | /about | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /posts /account /login |  |
| scaffold | /posts | renders and hydrates | JS | PASS | url /posts; hydrated true; h1 "Posts" |  |
| scaffold | /posts | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /account /login /posts/hello-world /posts/second-post |  |
| scaffold | /posts/hello-world | renders and hydrates | JS | PASS | url /posts/hello-world; hydrated true; h1 "Hello, world" |  |
| scaffold | /posts/hello-world | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /posts /account /login |  |
| scaffold | /posts/second-post | renders and hydrates | JS | PASS | url /posts/second-post; hydrated true; h1 "A second post" |  |
| scaffold | /posts/second-post | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /posts /account /login |  |
| scaffold | /login | renders and hydrates | JS | PASS | url /login; hydrated true; h1 "Sign in" |  |
| scaffold | /login | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /posts /account |  |
| scaffold | /account | renders and hydrates | JS | PASS | url /login?next=/account; hydrated true; h1 "Sign in" |  |
| scaffold | /account | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /posts /login |  |
| scaffold | /403 | renders and hydrates | JS | PASS | url /403; hydrated true; h1 "Access denied" |  |
| scaffold | /403 | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /posts /account /login |  |
| scaffold | /probe-form | renders and hydrates | JS | PASS | url /probe-form; hydrated true; h1 "Probe form" |  |
| scaffold | /probe-form | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /posts /account /login |  |
| scaffold | /probe-client | renders and hydrates | JS | PASS | url /probe-client; hydrated true; h1 "Probe client" |  |
| scaffold | /probe-client | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / /about /posts /account /login |  |
| scaffold | /login | after a native POST (422) form() hydrates from the server state (email kept, error shown) | JS | PASS | status 422; after hydration values {"email":"qa@example.com"} messages ["Wrong email or password"] |  |
| scaffold | /login | wrong credentials: double click sends one POST, error shown, no reload (matches no-JS message) | JS | PASS | POSTs 1 ["/_r/xq8y/login"]; full loads 0; messages ["Wrong email or password"] |  |
| scaffold | /login | right credentials: one POST, client navigation to `next` (/posts), probe survives | JS | PASS | POSTs 1; url /posts; probe kept; full loads 0; header "Signed in as qa@example.com" |  |
| scaffold | /probe-form | after a native POST (422) form() hydrates from the server state (values + messages) | JS | PASS | status 422; hydrated true; values {"name":"","email":"bad","age":"12"}; messages ["Required","Enter a valid email","Must be at least 18"] |  |
| scaffold | /probe-form | invalid: no POST, same messages as no-JS | JS | PASS | POSTs 0; url /probe-form (no-JS /probe-form); messages ["Required","Enter a valid email","Must be at least 18"] (no-JS ["Required","Enter a valid email","Must be at least 18"]); probe kept; full loads 0 |  |
| scaffold | /probe-form | server { errors }: one POST (double click), same field message as no-JS | JS | PASS | POSTs 1; url /probe-form (no-JS /probe-form); messages ["That name is taken"] (no-JS ["That name is taken"]); probe kept; full loads 0 |  |
| scaffold | /probe-form | server failure: one POST, same generic message as no-JS | JS | PASS | POSTs 1; url /probe-form (no-JS /probe-form); messages ["Something went wrong. Try again."] (no-JS ["Something went wrong. Try again."]); probe kept; full loads 0 |  |
| scaffold | /probe-form | { redirect: "/about" }: one POST, client navigation to /about like no-JS | JS | PASS | POSTs 1; url /about (no-JS /about); messages [] (no-JS []); probe kept; full loads 0 |  |
| scaffold | /probe-form | valid: one POST (double click), no error, stays on /probe-form like the no-JS PRG | JS | PASS | POSTs 1; url /probe-form (no-JS /probe-form); messages [] (no-JS []); probe kept; full loads 0 |  |
| scaffold | /probe-client | client mounts in place: no layout jump vs the placeholder | JS | PASS | canvas 320px mounted "yes"; #after top 573px (no-JS 573px, jump 0px); CLS 0.0000 |  |
| examples | / | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /; h1 "BRAINLLM examples"; main text 185 chars |  |
| examples | / | every internal link loads its page (2) | no-JS | PASS | / /dashboard |  |
| examples | /dashboard | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 200 url /dashboard; h1 "Users"; main text 145 chars |  |
| examples | /dashboard | every internal link loads its page (2) | no-JS | PASS | / /dashboard |  |
| examples | /no-such-page | renders meaningful HTML (h1 + main text, no spinner) | no-JS | PASS | status 404 url /no-such-page; h1 "Not found"; main text 9 chars |  |
| examples | / (todo) | client-only form (no `server:` submit) stays a plain form, not a POST | no-JS | PASS | method get; _brain "" |  |
| examples | / | renders and hydrates | JS | PASS | url /; hydrated true; h1 "BRAINLLM examples" |  |
| examples | / | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | /dashboard |  |
| examples | /dashboard | renders and hydrates | JS | PASS | url /dashboard; hydrated true; h1 "Users" |  |
| examples | /dashboard | links navigate on the client (window probe survives, no load event, same page as no-JS) | JS | PASS | / |  |
| examples | / (todo) | client-only form submits without a reload | JS | PASS | items ["buy milk"]; full loads 0 |  |
| run | cleanup | no process left | run | PASS | stopped 2 supervisors and their children |  |
