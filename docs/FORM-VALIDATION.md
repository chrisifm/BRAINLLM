# Native form validation

`brain-schema.js` is the shared rule implementation for browser forms, API shape validation and Node form posts. Form construction validates the complete nested schema before creating fields or running callbacks. Invalid rule syntax, unknown scalar types and incoherent bounds fail immediately.

## Rules and bounds

Supported scalar types are `str`, `num`, `bool`, `email`, `url`, `date`, `tel` and `file`. Required markers use `str!`; lists use `[str]`, `[str]!` or the existing `[str!]` spelling. Brackets must balance, and repeated required markers are invalid. A minimum/maximum can occur once each, and the minimum cannot exceed the maximum.

Bounds on strings and lists are nonnegative safe integers: character counts and item counts respectively. Numeric bounds can be finite signed decimals. Scalar boolean bounds are invalid. Nested schemas must be plain field objects; cyclic schemas, accessors, unsafe prototype-related field names, nesting beyond 16 and more than 4,096 fields are rejected. Scalar rule strings are limited to 256 characters.

## Values

Required text/numeric/date fields reject whitespace-only input. Optional blank input still skips other checks, preserving the existing optional-field contract. Nonblank string values are preserved rather than silently trimmed. String, email, URL and date fields require actual strings; objects, arrays, numbers and custom coercion objects cannot masquerade as text.

Numbers accept finite native numbers and decimal strings supplied by HTML controls, including a decimal fraction or exponent. Whitespace around a decimal token is allowed. Hexadecimal/binary strings, booleans, arrays, infinities and object coercion are rejected. Successful browser submits convert numeric strings and numeric list items to numbers. An optional blank number becomes `null`.

Boolean fields accept booleans, with missing optional input allowed. `bool!` still requires `true`, matching an agreement checkbox: `false` produces `Required`. Boolean lists validate boolean items, including `false`. Node HTML form posts recognize checkbox tokens `on`, `true`, `1`, `false` and `0`; arbitrary strings are rejected rather than coerced to false.

Dates use a strict calendar check. The supported strings are `YYYY-MM-DD` and a date followed by `THH:mm`, optional `:ss`/fractional seconds, and optional `Z` or numeric timezone offset. Month lengths, leap years and clock fields must be valid; `2026-02-30`, `1900-02-29` and `24:00` are rejected. Locale-dependent dates, numeric timestamps and Date objects are not schema date strings. This avoids engine-specific parsing and overflow normalization.

`tel` is a phone as people write it: an optional leading `+`, digits with spaces, dots, dashes or parentheses between them, and 7 to 15 digits (the E.164 maximum). It checks the shape, not whether the number exists or its country, and keeps the string as typed. Bounds count characters like `str`.

`file` holds a browser `File` (anything with a numeric `size` and a string `type`); its empty value is `null`. Bounds are bytes: `file!<=5000000`. `file(image/*,.pdf)` lists accepted MIME types, MIME groups or extensions, compared case-insensitively. In a list, `[file(.pdf)]!<=3` bounds the number of items and applies the accepted types to each; a per-item size belongs in `validate`. Nothing about a `File` is proof of its content: name and type come from the client, so the bucket or server function verifies again. Assign it with `on:change={e => f.v.doc = e.target.files[0] ?? null}`; `bind:value` does not apply to a file input. A `File` does not cross a server function: upload it to a declared bucket (`brain guide docs/backend/files.md`).

## Cross-field rules

`form(schema, { validate: v => ({ confirm: v.password === v.confirm ? '' : 'Passwords do not match' }) })` runs after every field passes its own rule, on submit only. It returns `{ field: message }`, or `null`; an empty message means valid. A non-empty message marks that field, focuses it and stops the submit. Repeat the rule on the server, which never runs this function.

## Submission contract

A locally invalid submit does not call cross-field validation, remote checks or the submit callback. It exposes field errors and retains the existing focus/ARIA behavior. The existing busy lock still prevents concurrent successful submissions. Node form posts validate raw decimal/boolean tokens before conversion so a malformed value cannot become a valid number or boolean before the shared rule sees it.

New fallback message keys `form.text` and `form.boolean` use the same dictionary override mechanism as existing validation messages. English remains the default.

This hardening deliberately rejects previously accepted malformed rules, impossible calendar dates and coercion-only scalar values. It does not change native HTTP error pages, authentication policy or browser storage. See the official [HTML date/number microsyntaxes](https://html.spec.whatwg.org/multipage/common-microsyntaxes.html) and [ECMAScript Date.parse contract](https://tc39.es/ecma262/multipage/numbers-and-dates.html#sec-date.parse) for the distinction between validating input and permissive date parsing.
