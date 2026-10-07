import { rewriteCss } from './css-rewrite.js';
import { logicalUrl } from './url-map.js';
import { wasm } from './rewriter.js';

// Adapt native stylesheet and declaration operations to logical resource addresses.
export function installCssRuntime(currentBaseUrl) {
  const views = new WeakMap();
  const originals = new WeakMap();
  const styleKeys = new Set(Object.getOwnPropertyNames(document.createElement('span').style));
  const sheetHref = Object.getOwnPropertyDescriptor(StyleSheet.prototype, 'href').get;
  const baseForSheet = sheet => sheet && sheetHref.call(sheet) ? logicalUrl(sheetHref.call(sheet)) : currentBaseUrl();
  const baseForStyle = style => baseForSheet(style.parentRule?.parentStyleSheet);
  const cssProperty = key => typeof key === 'string' && (styleKeys.has(key) || (!key.startsWith('--') && CSS.supports(key, 'initial')));
  const styleView = style => {
    if (!views.has(style)) {
      const view = new Proxy(style, {
        get(target, key) {
          const value = Reflect.get(target, key, target);
          return cssProperty(key) && typeof value === 'string' ? rewriteCss(value, baseForStyle(target), 'value', { reflect: true }) : value;
        },
        set(target, key, value) {
          return Reflect.set(target, key, cssProperty(key) ? rewriteCss(value, baseForStyle(target), 'value') : value, target);
        },
      });
      views.set(style, view);
      originals.set(view, style);
    }
    return views.get(style);
  };
  for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(CSSStyleDeclaration.prototype))) {
    if (typeof descriptor.value === 'function' && key !== 'constructor') {
      Object.defineProperty(CSSStyleDeclaration.prototype, key, { ...descriptor, value: function(...args) {
        const receiver = originals.get(this) ?? this;
        if (key === 'setProperty' && args.length > 1) args[1] = rewriteCss(args[1], baseForStyle(receiver), 'value');
        const result = descriptor.value.apply(receiver, args);
        return ['getPropertyValue', 'removeProperty'].includes(key) ? rewriteCss(result, baseForStyle(receiver), 'value', { reflect: true }) : result;
      } });
    } else if (key === 'cssText') {
      Object.defineProperty(CSSStyleDeclaration.prototype, key, {
        ...descriptor,
        get() {
          const receiver = originals.get(this) ?? this;
          return rewriteCss(descriptor.get.call(receiver), baseForStyle(receiver), 'declarationList', { reflect: true });
        },
        set(value) {
          const receiver = originals.get(this) ?? this;
          descriptor.set.call(receiver, rewriteCss(value, baseForStyle(receiver), 'declarationList'));
        },
      });
    }
  }
  const prototypes = [HTMLElement.prototype, SVGElement.prototype, globalThis.MathMLElement?.prototype, StyleSheet.prototype, CSSStyleSheet.prototype];
  for (const name of Object.getOwnPropertyNames(globalThis)) {
    if (/^CSS.*Rule$/.test(name) && globalThis[name]?.prototype) prototypes.push(globalThis[name].prototype);
  }
  for (const prototype of new Set(prototypes.filter(Boolean))) {
    for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(prototype))) {
      if (key === 'style' && descriptor.get) {
        Object.defineProperty(prototype, key, { ...descriptor, get() { return styleView(descriptor.get.call(this)); }, set: descriptor.set ? function(value) { descriptor.set.call(this, rewriteCss(value, currentBaseUrl(), 'declarationList')); } : undefined });
      } else if (['cssText', 'href'].includes(key) && descriptor.get) {
        Object.defineProperty(prototype, key, { ...descriptor, get() {
          const value = descriptor.get.call(this);
          if (key === 'href') return value ? logicalUrl(value) : value;
          return rewriteCss(value, currentBaseUrl(), 'stylesheet', { reflect: true });
        } });
      } else if (['insertRule', 'replace', 'replaceSync'].includes(key) && typeof descriptor.value === 'function') {
        Object.defineProperty(prototype, key, { ...descriptor, value: function(...args) {
          if (args.length) args[0] = rewriteCss(args[0], baseForSheet(this instanceof CSSStyleSheet ? this : this.parentStyleSheet), 'stylesheet', { documentBase: currentBaseUrl() });
          return descriptor.value.apply(this, args);
        } });
      }
    }
  }
  // Attribute selectors on URL attributes match the proxy URLs the DOM holds
  // (`docs/browser/preview.md` § Addresses and labels); the page reads back the selectors it wrote.
  const urlSelector = new RegExp(`\\[\\s*(${wasm.urlAttributes().join('|')})\\s*[\\^$*]?=`, 'i');
  const mappedSelectors = new Map();
  const mapSelector = selector => {
    const text = `${selector}`;
    if (!urlSelector.test(text)) return text;
    const base = currentBaseUrl();
    const key = `${base}\n${text}`;
    let mapped = mappedSelectors.get(key);
    if (mapped === undefined) {
      mapped = rewriteCss(text, base);
      // Pages query with few distinct selectors; a page that builds them endlessly starts over.
      if (mappedSelectors.size >= 1000) mappedSelectors.clear();
      mappedSelectors.set(key, mapped);
    }
    return mapped;
  };
  for (const prototype of [Document.prototype, DocumentFragment.prototype, Element.prototype]) {
    for (const name of ['querySelector', 'querySelectorAll', 'matches', 'closest', 'webkitMatchesSelector']) {
      const original = Object.getOwnPropertyDescriptor(prototype, name);
      if (!original) continue;
      Object.defineProperty(prototype, name, { ...original, value: { [name](selector, ...rest) { return original.value.call(this, mapSelector(selector), ...rest); } }[name] });
    }
  }
  for (const constructor of [globalThis.CSSStyleRule, globalThis.CSSPageRule]) {
    const descriptor = constructor && Object.getOwnPropertyDescriptor(constructor.prototype, 'selectorText');
    if (!descriptor) continue;
    Object.defineProperty(constructor.prototype, 'selectorText', {
      ...descriptor,
      get() { return rewriteCss(descriptor.get.call(this), currentBaseUrl(), 'selector', { reflect: true }); },
      set(value) { descriptor.set.call(this, mapSelector(value)); },
    });
  }
  const computedStyle = globalThis.getComputedStyle;
  globalThis.getComputedStyle = function(...args) { return styleView(computedStyle.apply(this, args)); };
}
