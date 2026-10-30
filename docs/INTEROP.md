# Foreign widget interoperability

`interop` gives an application-supplied DOM widget a dedicated subtree. The framework ships no React runtime, JSX parser, SDK loader or third-party bundler. Supply an adapter through an application `.jsbrain` module and use it inside a sized client-only component.

```html
<meta client height="320px">
<meta props="title:str">
<script>
  import adapter from '../lib/widget.jsbrain'
  let host = ref()
  const widget = interop(() => host, adapter, { title: props.title })
  watch(() => props.title, value => widget.update({ title: value }))
</script>
<div bind:ref={host} aria-label="Interactive widget"></div>
<style>
  div { min-height: 320px; }
</style>
```

`interop(elementOrResolver, adapter, initialProps, {onError}?)` must run during component setup, where an owner exists. Passing `() => host` resolves the bound element after insertion. Mount is deferred to a microtask and never runs during SSR or prerender. The component's declared height reserves layout space before mounting. Keep Brain markup outside the widget's subtree; the adapter exclusively owns its descendants and must not replace the boundary element.

The adapter contract is synchronous:

```js
export default {
  mount(element, props) {
    // SDK is supplied by the application and must already be available.
    const widget = externalSDK.create(element, props)
    return {
      update(nextProps) { widget.setOptions(nextProps) },
      destroy() { widget.destroy() }
    }
  }
}
```

`mount` returns a cleanup function or `{update?, destroy}`. `destroy` is mandatory and runs at most once on owner disposal or `controller.destroy()`. Unmount before the mounting microtask cancels mounting. Updates before mounting replace pending props; subsequent updates pass the complete next props object to the adapter. There is no implicit merge, signal traversal, remount or deep comparison. Call `watch` explicitly for reactive updates. A cleanup-only adapter rejects updates after mount; an already destroyed controller rejects every update.

Exceptions from mount, update and cleanup go to `onError(error, phase)` when supplied, otherwise to the owning component's runtime error boundary. Phases are `mount`, `update`, and `destroy`. A mount that partially allocates and then throws must clean up its own partial resources before throwing. Asynchronous mount returns are rejected; await SDK loading in an application module or an enclosing loading state before creating this boundary. The framework cannot cancel resources hidden behind a rejected asynchronous adapter.

## React adapter without JSX

An application can provide React and `createRoot` through an installed SDK or its own approved module distribution. An adapter then calls the supplied functions; BRAINLLM does not parse or bundle them:

```js
export default {
  mount(element, props) {
    const root = createRoot(element)
    root.render(React.createElement(ExistingWidget, props))
    return {
      update(nextProps) {
        root.render(React.createElement(ExistingWidget, nextProps))
      },
      destroy() { root.unmount() }
    }
  }
}
```

The application owns SDK versioning, credentials, network policies, readiness, accessibility, fallback UI and any external script integrity checks. Do not interpolate SDK output as HTML, expose session secrets as widget props or let a foreign widget mutate surrounding Brain-managed nodes. Pass data and explicit callbacks across the boundary. This interface also fits maps, editors, charts and payment widgets whose SDKs expose a mount/update/destroy lifecycle.
