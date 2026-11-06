# Browser defaults for generated themes

New projects include conservative print and forced-color rules in the low-priority reset layer of `tokens.cssbrain`. Authored components and later theme layers can override them. Upgrading the compiler does not rewrite an existing project's theme automatically.

In forced-colors mode, focus outlines use `Highlight`, controls use `ButtonText` borders, and links use `LinkText`. The browser remains responsible for adapting the page palette; the template does not disable forced-color adjustment. Keep meaningful focus and control boundaries when customizing a theme.

For printing, the starter requests a light color scheme, removes the body's screen minimum height, background and padding, underlines links, and avoids breaks inside images, SVGs and quotations. These are baseline formatting rules, not a guarantee that every authored layout paginates well. Review real multi-page content and print output after changing layouts.

`content-visibility` is not enabled globally. Hidden rendering can affect intrinsic sizes, scrolling, focus and browser find behavior. Authors can opt measured, independent below-fold sections into supported CSS, with an appropriate intrinsic placeholder:

```css
@supports (content-visibility: auto) {
  .archive-section {
    content-visibility: auto;
    contain-intrinsic-size: auto 40rem;
  }
}
```

Choose a placeholder from the actual layout, preserve accessible reading/focus order, and test target browsers. Unsupported browsers retain normal rendering. The generated-theme browser test exercises screen, print, forced-colors and explicit opt-in behavior in an owned headless browser; it is not an accessibility certification or a substitute for testing an application's content.
