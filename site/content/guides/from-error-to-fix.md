---
title: From error to fix with brain check
description: What a BRAINLLM diagnostic contains, when brain fix can apply it for you and how to handle the ones that need judgment.
date: 2026-10-13
tags: [cli, errors]
author: BRAINLLM team
---
Every problem the checker finds is an instruction, not a puzzle. It names the rule, the place and the correction, so a model can fix it without guessing.

## Anatomy of a diagnostic

```json
{"code":"B0113","file":"comp/list.brain","line":4,"col":3,
 "msg":"items.push mutates a sig in place",
 "fix":"copy and reassign: items = [...items, item]",
 "edits":[{"line":4,"col":3,"endLine":4,"endCol":18,"text":"items = [...items, item]"}]}
```

The code tells you the family: B00 structure, B01 script, B02 markup, B03 CSS, B04 accessibility, B05 SEO and B07 server and secrets. `brain explain B0113` prints why the rule exists and a correct example.

## Exact edits

When the correction is unique, the diagnostic carries `edits`: exact replacements by line and column. `brain fix` applies all of them and checks again, so run it before editing by hand. Normalizations that are always safe, such as replacing `var` with `let`, are applied by the compiler on its own.

## Judgment calls

Some fixes need a decision: the alt text of an image, the wording of a label, which heading level a section deserves. Those never get automatic edits. The message says what is missing and the fix says what a good answer looks like; the model, or you, writes it.

## When the same error comes back

If a diagnostic fails the same way three checks in a row, it comes back with `"stuck": true`. That is the signal to stop patching: read its `explain` field, or run `brain explain <code>`, and rewrite that part from the correct example instead of nudging the same lines again. Commands follow the same idea when they fail on their own: they exit with code 3 and print the next command to run.

## The loop

1. `brain fix`
2. `brain check --json`
3. apply each remaining fix literally
4. repeat until the list is empty, then run `brain doctor`

Following the loop literally is faster than reasoning about each error, and it is what the checker was designed for.
