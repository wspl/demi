import { rewriteJavaScript } from './rewrite.js';
import { rewriteImportMap } from './module-specifiers.js';
import { rewriteCss } from './css-rewrite.js';
import { rewriteResourceAttribute } from './resource-attributes.js';

// Prepare script and style text before DOM insertion can execute it or fetch resources.
export function installDomTextRuntime(currentBaseUrl) {
  const textContent = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent');
  const scriptText = Object.getOwnPropertyDescriptor(HTMLScriptElement.prototype, 'text');
  const characterData = Object.getOwnPropertyDescriptor(CharacterData.prototype, 'data');
  const prepared = new WeakMap();
  const textElement = node => node instanceof HTMLScriptElement || node instanceof HTMLStyleElement;
  const scriptTypes = ['', 'module', 'text/javascript', 'application/javascript'];
  // Code that does not parse stays as written: the browser reports its error when it runs, as
  // it would natively, and text that is not code (a shader in a script element) is untouched.
  function script(source) {
    try {
      return rewriteJavaScript(source, 'inline.js', { base: currentBaseUrl() });
    } catch {
      return source;
    }
  }

  // Keep application source text distinct from the bytes passed to native parsers.
  // `inserting`: the element is about to be inserted (see prepare).
  function rewrite(node, source, inserting = false) {
    const previous = prepared.get(node);
    if (source === previous?.output) source = previous.source;
    let output = source;
    if (node instanceof HTMLStyleElement) output = rewriteCss(source, currentBaseUrl());
    // A script runs, as what its type then says, only once it is in a document: a detached one
    // is rewritten when it is inserted (see prepare), after the page has set its type.
    else if (node instanceof HTMLScriptElement && !node.hasAttribute('src') && (node.isConnected || inserting)) {
      if (node.type === 'importmap') output = rewriteImportMap(source, currentBaseUrl());
      else if (scriptTypes.includes(node.type) && source.trim()) output = script(source);
    }
    return { source, output };
  }
  Object.defineProperty(Node.prototype, 'textContent', {
    ...textContent,
    get() {
      const value = textContent.get.call(this);
      const previous = prepared.get(this);
      if (value === previous?.output) return previous.source;
      return this instanceof HTMLStyleElement ? rewriteCss(value, currentBaseUrl(), 'stylesheet', { reflect: true }) : value;
    },
    set(value) {
      const text = textElement(this) && value != null ? rewrite(this, `${value}`) : undefined;
      textContent.set.call(this, text ? text.output : value);
      if (text) prepared.set(this, text);
      else prepared.delete(this);
    },
  });
  for (const property of ['text', 'textContent', 'innerText']) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLScriptElement.prototype, property);
    if (!descriptor) continue;
    Object.defineProperty(HTMLScriptElement.prototype, property, {
      ...descriptor,
      get() {
        const value = descriptor.get.call(this);
        const previous = prepared.get(this);
        return value === previous?.output ? previous.source : value;
      },
      set(value) {
        const source = property === 'textContent' && value == null ? '' : `${value}`;
        const text = rewrite(this, source);
        descriptor.set.call(this, text.output);
        prepared.set(this, text);
      },
    });
  }

  // Prepare detached policy and text nodes before insertion; retain Text node identities.
  function prepare(node) {
    if (!(node instanceof Node) || node.isConnected) return;
    if (node instanceof HTMLMetaElement && node.hasAttribute('http-equiv')) {
      const value = node.getAttribute('http-equiv');
      node.setAttribute('http-equiv', rewriteResourceAttribute('http-equiv', value, currentBaseUrl()));
    }
    if (textElement(node)) {
      const source = node instanceof HTMLScriptElement ? scriptText.get.call(node) : textContent.get.call(node);
      const text = rewrite(node, source, true);
      if (text.output !== source) {
        const children = [...node.childNodes].filter(child => child.nodeType === Node.TEXT_NODE || child.nodeType === Node.CDATA_SECTION_NODE);
        for (let index = 0; index < children.length; index++) characterData.set.call(children[index], index === 0 ? text.output : '');
      }
      prepared.set(node, text);
      return;
    }
    for (const child of node.childNodes) prepare(child);
  }
  // Text added to a connected <style> or inline <script> takes effect as it is
  // inserted, as webpack's style-loader does with appendChild(createTextNode(css)).
  // A detached element is prepared whole when it is inserted (see prepare).
  function prepareChild(parent, node) {
    if (!textElement(parent) || !parent.isConnected || !(node instanceof CharacterData) || node.isConnected || node instanceof Comment) return;
    if (parent instanceof HTMLScriptElement && (parent.hasAttribute('src') || !scriptTypes.includes(parent.type))) return;
    const source = characterData.get.call(node);
    if (!source.trim()) return;
    const output = parent instanceof HTMLStyleElement ? rewriteCss(source, currentBaseUrl()) : script(source);
    if (output !== source) characterData.set.call(node, output);
  }
  return { prepare, prepareChild, rewrite, textElement, remember(node, text) { prepared.set(node, text); } };
}
