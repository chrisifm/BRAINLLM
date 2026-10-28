---
title: Build your first app with a language model
description: Create a BRAINLLM app, hand it to any LLM with the starter prompt and let it build views until brain doctor exits with zero.
date: 2026-10-13
tags: [getting started, agents]
author: BRAINLLM team
---
A new BRAINLLM app is ready for a model the moment it is created: the contract is one command away, every file kind has a generator, and the checker tells the model exactly what to change.

## Create the app

```sh
brain new my-app
cd my-app
brain run
```

The app opens with a home page that shows a starter prompt. It already has a layout in `app.brain`, design tokens, an about page, a not-found page and icons.

## Give the model the contract

Paste the starter prompt into the model of your choice and describe the app you want. The prompt asks the model to read `brain spec` fully before writing anything, because nothing outside that document exists. Models that guess APIs are the main source of broken code; a short, complete contract removes the guessing.

## Let it work in the loop

The model adds files with `brain add`, edits them, runs `brain fix` and then `brain check --json`, and applies each remaining fix literally:

```json
{"code":"B0202","file":"comp/todo.brain","line":14,"col":3,
 "msg":"each without key",
 "fix":"add a key: {#each items as it (it.id)}"}
```

It repeats until the check returns an empty list, and finishes when `brain doctor` exits with zero. If you connect the model through the [Model Context Protocol](https://modelcontextprotocol.io/), it can run every command as a tool, including starting the dev server and taking screenshots.

## Keep the model on the rails

Every new app includes an `AGENTS.md` with the same rules for any model. Before importing anything, the model reads `brain map --json`, which lists the routes, components, props and modules that exist, and it uses `brain card <name>` to learn a component's contract without reading its body. A component that grows past 150 lines gets B1010, and `brain split` moves the part it suggests into a new component, refusing the change if the check would fail.

## What to check yourself

Open the pages, try the flows and read the copy. The framework guarantees structure, accessibility and SEO; it cannot know whether the app does what you meant. When something is off, describe the problem to the model in plain words and let it go through the loop again.
