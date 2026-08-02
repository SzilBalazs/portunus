// Sandboxed-iframe document scaffolds.
//
// Two consumers with deliberately different postures:
//
//  - `buildSrcdoc` (extension `html` previews) - `sandbox=""`, no scripting at
//    all, and a small utility stylesheet so extension authors get the host's
//    typography for free.
//  - `buildOfficeSrcdoc` (rendered office documents) - `sandbox="allow-scripts"`
//    with a nonce'd bootstrap, because a document preview needs to scroll to its
//    match and answer the host's keyboard.
//
// They share only `themeVarDecls` and `SANDBOX_SCROLLBAR_CSS`. The CSP, the base
// stylesheet and the utility classes are per-consumer on purpose.

// ── shared ───────────────────────────────────────────────────────────────────

/**
 * Host scrollbar look (App.css .text-preview-wrap), for documents long enough
 * to scroll. Only ::-webkit-* rules:
 * scrollbar-width/color would override them on WebKitGTK.
 */
export const SANDBOX_SCROLLBAR_CSS =
  `::-webkit-scrollbar{width:10px;height:10px}` +
  `::-webkit-scrollbar-thumb{background:var(--bg-input);border-radius:5px;` +
  `border:2px solid transparent;background-clip:padding-box;min-height:32px;min-width:32px}` +
  `::-webkit-scrollbar-thumb:hover{background:var(--fg-mute);background-clip:padding-box}` +
  `::-webkit-scrollbar-track{background:transparent}` +
  `::-webkit-scrollbar-corner{background:transparent}`;

/**
 * `--name:value;...` for the named custom properties, read off the host root.
 *
 * Unset vars are dropped rather than emitted empty: an empty custom property
 * makes `var(--x, fallback)` resolve to nothing instead of the fallback.
 */
export function themeVarDecls(names: string[]): string {
  const style = getComputedStyle(document.documentElement);
  return names
    .map(v => [v, style.getPropertyValue(v).trim()] as const)
    .filter(([, value]) => value)
    .map(([v, value]) => `${v}:${value}`)
    .join(';');
}

/**
 * 32 random bytes, base64. Used for both the script nonce and the message token:
 * generated per document by the host and never derived from content, so neither
 * can be predicted or replayed by anything the document happens to contain.
 */
export function randomToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

// ── extension `html` previews ────────────────────────────────────────────────

const THEME_VARS = [
  '--fg', '--fg-mute', '--fg-dim', '--fg-desc',
  '--bg', '--bg-deep', '--bg-card',
  '--accent', '--accent-soft', '--accent-border',
  '--radius', '--radius-sm', '--line', '--border', '--text-on-accent',
  // Accent-bleed: the selected result's sampled color, set on documentElement by
  // App.tsx. Flows the album-art / icon hue into the sandboxed preview HTML.
  '--item-accent', '--item-on-accent',
  // Scrollbar thumb, so the iframe's own scrollbars match the host's.
  '--bg-input',
  // UI scale factor. The frame element is unzoomed by --ui-zoom-inv (App.css),
  // so the document re-applies the zoom itself.
  '--ui-zoom',
];

const EXT_UTILS_CSS = [
  '.text-mute{color:var(--fg-mute)}.text-dim{color:var(--fg-dim)}',
  '.text-desc{color:var(--fg-desc)}.text-accent{color:var(--accent)}',
  '.text-xs{font-size:10px;letter-spacing:.04em}.text-sm{font-size:11px}',
  '.text-lg{font-size:16px}.text-hero{font-size:42px;font-weight:200;line-height:1.1}',
  '.text-label{font-size:10px;text-transform:uppercase;letter-spacing:.07em;color:var(--fg-mute)}',
  '.mono{font-family:ui-monospace,"SF Mono","Fira Code",monospace;font-size:12px}',
  '.truncate{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}',
  '.row{display:flex;align-items:center;gap:8px}',
  '.col{display:flex;flex-direction:column;gap:6px}',
  '.fill{flex:1;min-width:0}.between{justify-content:space-between}.wrap{flex-wrap:wrap}',
  '.card{background:var(--bg-card);border-radius:var(--radius-sm);padding:10px 12px}',
  '.surface{background:var(--bg-deep);border-radius:var(--radius-sm);padding:10px 12px}',
  '.divider{height:1px;border:none;background:var(--line);margin:6px 0}',
  '.tag{display:inline-block;font-size:10px;background:var(--accent-soft);border-radius:3px;padding:1px 5px;color:var(--fg-mute)}',
  '.tag-accent{display:inline-block;font-size:10px;background:var(--accent);border-radius:3px;padding:1px 5px;color:var(--text-on-accent)}',
  '.bar{height:3px;border-radius:2px;background:var(--accent)}',
  '.accent-line{border-left:2px solid var(--accent-border);padding-left:8px}',
].join('');

/**
 * Document scaffold for an `html` preview.
 *
 * `:root{zoom}` mirrors the launcher's own UI scale, which App.css cancels on
 * the frame element - see the `.ext-preview-html` comment for why the frame
 * itself must not be zoomed by its parent document.
 *
 * Two rules that must not be reintroduced: no percentage/viewport heights, and
 * no `overflow` on `body`. A body locked to the viewport height keeps that
 * height when a horizontal scrollbar appears, so it overflows by exactly the
 * scrollbar's thickness and grows a spurious vertical one; and `overflow` on
 * body adds a second scroll container beside the viewport's. Body is
 * content-sized, the viewport is the only scroller, nothing is ever clipped.
 */
export function buildSrcdoc(content: string): string {
  const vars = themeVarDecls(THEME_VARS);
  return (
    `<!DOCTYPE html><html><head>` +
    `<meta http-equiv="Content-Security-Policy" ` +
    `content="default-src 'none'; style-src 'unsafe-inline' data:; img-src data:;">` +
    `<style>:root{${vars};zoom:var(--ui-zoom,1)}` +
    `*{box-sizing:border-box;margin:0;padding:0}` +
    `body{background:transparent;color:var(--fg);font-size:13px;line-height:1.5;` +
    `font-family:system-ui,-apple-system,sans-serif}` +
    SANDBOX_SCROLLBAR_CSS +
    `${EXT_UTILS_CSS}</style>` +
    `</head><body>${content}</body></html>`
  );
}

// ── rendered office documents ────────────────────────────────────────────────

/** Which renderer produced the document (mirrors the backend `Shape`). */
export type OfficeVariant = 'doc' | 'sheet' | 'slide';

export interface OfficeSrcdocOpts {
  /** Per-document random token every inbound message must carry. */
  token: string;
  /** `id` of the mark to centre before `ready` (OfficeDoc.bestMarkId). */
  bestMarkId?: string | null;
  /** Start with matched-term highlighting suppressed (Ctrl+H is off). */
  hlOff?: boolean;
  /** Slide canvas size in CSS px (OfficeDoc.natural). */
  natural?: [number, number] | null;
  /** docx page width + padding in CSS px (OfficeDoc.page). */
  page?: [number, number, number] | null;
  /**
   * Reader zoom to open at. Baked in rather than sent after `ready` so a sheet
   * switch or a new file does not visibly snap from 100% back to the user's zoom.
   */
  zoom?: number;
}

/** Wrapper the reader's zoom transform is applied to. */
const OFFICE_ZOOM_ID = 'ozoom';

/** Reader zoom bounds. Below 0.4 a sheet is unreadable; above 3 nothing fits. */
export const OFFICE_ZOOM_MIN = 0.4;
export const OFFICE_ZOOM_MAX = 3;
export const OFFICE_ZOOM_STEP = 1.1;

export function clampOfficeZoom(z: number): number {
  if (!Number.isFinite(z)) return 1;
  return Math.round(Math.min(OFFICE_ZOOM_MAX, Math.max(OFFICE_ZOOM_MIN, z)) * 100) / 100;
}

/**
 * Custom properties an office document may read. Deliberately narrower than
 * THEME_VARS: the *chrome* is themed (row/column headers, notes, scrollbars,
 * match highlights), the *paper* is not. A document authors its own ink and
 * fills, and recolouring those to match the launcher's palette would
 * misrepresent the file's contents.
 */
const OFFICE_THEME_VARS = [
  '--fg', '--fg-mute', '--fg-dim',
  '--bg-preview', '--bg-deep', '--bg-card', '--bg-input',
  '--accent', '--line', '--border', '--radius-sm',
  '--ui-zoom',
];

/**
 * Structural CSS shared by all three office variants.
 *
 * Every selector is a single class or element so the document's own per-format
 * rules (emitted after this block, at equal specificity) win on source order -
 * the same discipline the backend's BASE_CSS keeps.
 */
const OFFICE_BASE_CSS =
  `*{box-sizing:border-box;margin:0;padding:0}` +
  // Match highlighting. Mirrors `mark.preview-hl` in App.css - #fff is ink over
  // the accent fill (as it is there), not a theme surface. No padding: office
  // cells are `white-space:pre;overflow:hidden`, so a mark that grows the inline
  // box would shift the grid.
  `mark.preview-hl{background:color-mix(in srgb,var(--accent) 70%,transparent);` +
  `color:#fff;font-weight:600;border-radius:2px}` +
  // Ctrl+H toggle: a class flip on <html>, so it costs no reload and no reflow
  // of the document's own styles.
  `html.hl-off mark.preview-hl{background:none;color:inherit;font-weight:inherit}`;

/**
 * Per-variant scaffold.
 *
 * The sheet variant keeps the `buildSrcdoc` invariants verbatim: body is
 * content-sized, the viewport is the only scroller, and body carries no
 * `overflow`. The page column / sheet surround sits on `--bg-preview` so the
 * scrollbar track runs over chrome rather than over paper, which is what lets
 * SANDBOX_SCROLLBAR_CSS work unmodified.
 *
 * `--office-zoom` is the reader's own zoom factor. It is applied as a *transform*
 * on body, not as `zoom`, and that distinction is load-bearing:
 *
 * `zoom` scales computed font-size, and WebCore then re-applies its "smart
 * minimum" to the result (`computedFontSizeFromSpecifiedSize`: a size that was
 * legible before zoom is floored at `minimumLogicalFontSize`, default 9px). The
 * sheet's base font is 14.667px, so every zoom below ~0.61 produced the same 9px
 * text while the boxes around it kept shrinking - zooming out stopped doing
 * anything to the text and the grid just got cramped. WebKitGTK exposes only the
 * *hard* minimum (`minimum-font-size`, already 0); the smart minimum has no
 * public setter, so the only fix is not to route the reader's zoom through
 * font-size at all.
 *
 * `transform` is purely geometric and therefore exactly linear. The blurry-raster
 * concern that rules transforms out elsewhere applies to transforming the iframe
 * *element* (a replaced element, so its composited layer gets blitted); a
 * transform inside the document rasterizes text at the final device scale.
 *
 * `width: calc(100% / z)` is the other half: transform does not affect layout, so
 * the body has to be laid out at the pre-scale width for the scaled result to
 * come out one viewport wide.
 *
 * The launcher's own `--ui-zoom` stays on `zoom` - it is the UI scale the rest of
 * the app uses, and it is not a per-document control.
 */
function officeVariantCss(variant: OfficeVariant, opts: OfficeSrcdocOpts): string {
  // The transform goes on the wrapper rather than on body: body's overflow
  // propagates to the viewport, and a transformed body is exactly the case where
  // that propagation gets murky. An ordinary block in between keeps the viewport
  // an ordinary scroller.
  const root =
    `:root{--office-zoom:${clampOfficeZoom(opts.zoom ?? 1)};zoom:var(--ui-zoom,1)}` +
    `body{background:var(--bg-preview)}` +
    `#${OFFICE_ZOOM_ID}{transform-origin:0 0;transform:scale(var(--office-zoom,1))}`;
  switch (variant) {
    case 'sheet':
      // No `width: calc(100% / z)` here, and that omission is the difference
      // between a smooth zoom and an unusable one. A width that depends on the
      // zoom dirties layout on every step, and a 1000x30 sheet is 30k cells to
      // relay out per wheel tick; a bare transform touches no layout at all.
      //
      // Sheets can afford to skip it because their content is content-sized: the
      // grid is a `table-layout:fixed` table of explicit column widths, so it
      // never needed a percentage width to size against. `max-content` is also
      // free of any dependency on body's width, which matters because the
      // bootstrap sizes body from the *scaled* wrapper (transforms do not affect
      // layout, so without that body keeps its full unscaled box and the viewport
      // scrolls over a blank region) - a percentage here would make that circular.
      return root + `#${OFFICE_ZOOM_ID}{width:max-content}body{cursor:grab}`;
    case 'doc': {
      // Provisional until the docx renderer lands (backend stage 2): page geometry
      // arrives as [width, padX, padY] in CSS px, centred on the chrome backdrop.
      const [w, px, py] = opts.page ?? [816, 96, 96];
      // These two keep the zoom-dependent width the sheet drops: their content
      // *is* sized against the viewport (a centred page column, a letterboxed
      // canvas), and the relayout it costs is one box, not a grid of cells.
      return (
        root +
        `#${OFFICE_ZOOM_ID}{display:flex;justify-content:center;` +
        `width:calc(100% / var(--office-zoom,1))}` +
        `.xl-doc,.od-doc{width:${w}px;max-width:100%;padding:${py}px ${px}px;background:#fff;color:#000}`
      );
    }
    case 'slide': {
      // Provisional until the pptx renderer lands (backend stage 3): a fixed
      // canvas, centred in the (pre-scale) viewport.
      const [w, h] = opts.natural ?? [960, 540];
      return (
        root +
        `#${OFFICE_ZOOM_ID}{display:flex;align-items:center;justify-content:center;` +
        `width:calc(100% / var(--office-zoom,1));min-height:calc(100vh / var(--office-zoom,1))}` +
        `.xl-doc,.od-slide{position:relative;width:${w}px;height:${h}px;background:#fff;color:#000}`
      );
    }
  }
}

/**
 * The one piece of trusted script in an office preview, emitted here rather than
 * by the Rust renderer so it lives in a single reviewable place: the renderer
 * only ever produces inert markup, and any script that shows up in a document is
 * by definition an escaping bug (and is blocked by the nonce'd CSP).
 *
 * Runs last in <body>, so the document is parsed by the time it executes. It
 * centres the best match *before* posting `ready`, which is the host's cue to
 * reveal the buffer - the match is on screen in the first painted frame.
 *
 * It also owns two things the host cannot reach across the frame boundary:
 * *focus custody* (a click inside a subframe moves focus there, and the host's
 * card-level `mousedown` preventDefault never sees it) and *ctrl+wheel zoom*
 * (the wheel event is delivered to the frame, not to the host).
 */
function officeBootstrap(variant: OfficeVariant, opts: OfficeSrcdocOpts): string {
  const token = JSON.stringify(opts.token);
  const mark = JSON.stringify(opts.bestMarkId ?? null);
  const zoom = clampOfficeZoom(opts.zoom ?? 1);
  // Only the sheet wrapper is laid out independently of body's width, so only it
  // can have body sized from it without going circular. The other two variants
  // still carry the zoom-dependent width and size themselves.
  const fits = variant === 'sheet';
  return (
    `(function(){` +
    `var T=${token};` +
    // `parent` is the app origin; targetOrigin '*' because this document's own
    // origin is opaque and there is nothing here worth withholding - the token is
    // what authenticates traffic in the other direction.
    `var post=function(m){m.token=T;parent.postMessage(m,'*')};` +
    `var root=document.documentElement;` +
    `var vp=function(){return document.scrollingElement||document.documentElement};` +
    // Horizontal scrolling may not belong to the viewport: a sheet keeps its grid
    // in an inner scroller so frozen panes have something to stick to. Bounded
    // breadth-first walk from body rather than a class name, so this
    // stays true for the doc and slide renderers too - and rather than
    // querySelectorAll('*'), which on a 50k-cell sheet is not free. Depth 4
    // because the zoom wrapper adds a level between body and the document.
    `var hz=function(){` +
    `var v=vp();if(v.scrollWidth>v.clientWidth)return v;` +
    `var q=[document.body],d=0;` +
    `while(q.length&&d<4){var n=[];for(var i=0;i<q.length;i++){` +
    `var c=q[i].children;for(var j=0;j<c.length;j++){` +
    `if(c[j].scrollWidth>c[j].clientWidth+1)return c[j];n.push(c[j]);}}` +
    `q=n;d++;}` +
    `return v;};` +
    // ── zoom ──
    // One custom property drives the wrapper's scale transform - see
    // officeVariantCss for why it is a transform and not `zoom`. Kept in the frame
    // rather than on the iframe element (transforming a replaced element blits its
    // composited raster; transforming a box inside the document does not). The
    // host mirrors the value so it survives a document swap, so every change is
    // echoed back as `zoomed`.
    `var W=document.getElementById(${JSON.stringify(OFFICE_ZOOM_ID)});` +
    `var Z=${zoom},ZMIN=${OFFICE_ZOOM_MIN},ZMAX=${OFFICE_ZOOM_MAX},raf=0;` +
    // A transform does not affect layout, so body keeps the *unscaled* box of its
    // content and the viewport happily scrolls over the empty difference. Sizing
    // body to the scaled wrapper is what removes that phantom region.
    // offsetWidth/offsetHeight are the untransformed layout box, which is exactly
    // the number to multiply.
    (fits
      ? `var fit=function(){if(!W)return;var b=document.body.style;` +
        `b.width=Math.ceil(W.offsetWidth*Z)+'px';` +
        `b.height=Math.ceil(W.offsetHeight*Z)+'px';};`
      : `var fit=function(){};`) +
    // A trackpad emits wheel events far faster than the compositor draws, so the
    // write is coalesced to one per frame. Z is updated synchronously (and echoed
    // straight away) so the accumulated factor and the host's badge never lag
    // behind the gesture - only the style write waits.
    `var flush=function(){raf=0;root.style.setProperty('--office-zoom',String(Z));fit();};` +
    `var setZoom=function(z,rid){` +
    `z=Math.round(Math.max(ZMIN,Math.min(ZMAX,z||1))*100)/100;` +
    `Z=z;if(!raf)raf=requestAnimationFrame(flush);` +
    `post({type:'zoomed',factor:z,requestId:rid});};` +
    `window.addEventListener('wheel',function(e){` +
    `if(!e.ctrlKey)return;e.preventDefault();` +
    `setZoom(Z*(e.deltaY<0?${OFFICE_ZOOM_STEP}:1/${OFFICE_ZOOM_STEP}));` +
    `},{passive:false});` +
    // Backstop for the case where focus did end up inside the frame despite the
    // custody rules below - the host's own ctrl+/-/0 listener would never fire.
    `window.addEventListener('keydown',function(e){` +
    `if(!e.ctrlKey||e.altKey||e.metaKey)return;var k=e.key;` +
    `if(k==='='||k==='+')setZoom(Z*${OFFICE_ZOOM_STEP});` +
    `else if(k==='-'||k==='_')setZoom(Z/${OFFICE_ZOOM_STEP});` +
    `else if(k==='0')setZoom(1);else return;` +
    `e.preventDefault();});` +
    // ── focus custody ──
    // The launcher pins focus to the search input (App.tsx cancels every
    // mousedown that would move it). A subframe breaks that: its mousedown is
    // dispatched inside *this* document, so the host never sees it and the click
    // silently focuses the frame, after which every keybind is dead. Cancel it
    // here, with the same rule and for the same reason. WebKit does not dispatch
    // mousedown for scrollbar hits, so dragging a scrollbar is unaffected.
    //
    // Cancelling the default also frees the primary button up: with no caret and
    // no native selection to place, a press-and-move is unambiguously a pan.
    `var drag=null;` +
    `var endDrag=function(){drag=null;document.body.style.cursor='';};` +
    `document.addEventListener('mousedown',function(e){e.preventDefault();` +
    `if(e.button!==0)return;drag={x:e.clientX,y:e.clientY};` +
    `document.body.style.cursor='grabbing';},true);` +
    `document.addEventListener('mousemove',function(e){` +
    `if(!drag)return;` +
    // Raw pointer deltas, not scaled by Z: the content moves with the cursor, so
    // the grab point stays under it whatever the zoom.
    `vp().scrollTop-=e.clientY-drag.y;hz().scrollLeft-=e.clientX-drag.x;` +
    `drag.x=e.clientX;drag.y=e.clientY;});` +
    `document.addEventListener('mouseup',endDrag);` +
    // A release outside the frame never arrives as a mouseup here, and a drag that
    // outlives the button is a cursor stuck on 'grabbing'.
    `document.addEventListener('mouseleave',endDrag);` +
    `window.addEventListener('blur',endDrag);` +
    // …and if focus lands here anyway (a route preventDefault does not cover,
    // e.g. the frame being tabbed into), hand it straight back.
    `window.addEventListener('focus',function(){post({type:'refocus'});});` +
    `window.addEventListener('message',function(e){` +
    // e.origin is the literal string "null" for an opaque origin and so proves
    // nothing; identity comes from the sender being our parent plus the token.
    `if(e.source!==parent)return;` +
    `var d=e.data;if(!d||typeof d!=='object'||d.token!==T)return;` +
    `var s=vp();` +
    `switch(d.type){` +
    `case 'scrollBy':` +
    `s.scrollTop+=(d.dy||0)+(d.pages||0)*s.clientHeight*0.9;` +
    `if(d.dx)hz().scrollLeft+=d.dx;break;` +
    `case 'scrollTo':` +
    `s.scrollTop=d.top==='end'?s.scrollHeight:d.top==='start'?0:(+d.top||0);break;` +
    `case 'hl':root.classList.toggle('hl-off',!d.on);break;` +
    `case 'section':s.scrollTop=0;s.scrollLeft=0;break;` +
    `case 'zoom':setZoom(+d.factor||1,d.requestId);break;` +
    `}` +
    `});` +
    `document.addEventListener('selectionchange',function(){` +
    `var sel=document.getSelection();post({type:'selection',text:sel?String(sel):''});` +
    `});` +
    // Body has to be sized before the match is centred: scrollIntoView against a
    // viewport whose scroll range is still the unscaled one lands in the wrong
    // place.
    `fit();` +
    `var m=${mark};var el=m?document.getElementById(m):null;` +
    `if(el&&el.scrollIntoView)el.scrollIntoView({block:'center',inline:'center'});` +
    `post({type:'ready'});` +
    `})();`
  );
}

/**
 * Document scaffold for a rendered office document.
 *
 * Sandbox posture, deliberately different from the extension previews and worth
 * understanding before loosening it:
 *
 *  - `sandbox="allow-scripts"` and never `allow-same-origin`. That *pair* is the
 *    sandbox escape - a same-origin scripted frame can reach the parent DOM. With
 *    scripts alone the origin stays opaque: no parent DOM, no app-origin storage,
 *    no Tauri IPC bridge. Everything the frame can do it does by postMessage,
 *    through the small protocol above.
 *  - `script-src 'nonce-…'` with 32 fresh random bytes per document, generated by
 *    the host and never derived from content. So a `<script>` that reached the
 *    markup through an escaping bug carries no nonce and never runs, and the same
 *    CSP kills `on*=` handlers and `javascript:` URLs.
 *  - `style-src 'unsafe-inline'` with no nonce, on purpose: office HTML is dense
 *    with `style=` attributes, and adding a nonce to style-src would *disable*
 *    'unsafe-inline' and strip every one of them.
 *  - `default-src 'none'`, `img-src data:` (media is inlined by the renderer),
 *    `form-action 'none'`, `base-uri 'none'` - nothing loads off the network.
 */
export function buildOfficeSrcdoc(
  html: string,
  variant: OfficeVariant,
  opts: OfficeSrcdocOpts,
): string {
  const vars = themeVarDecls(OFFICE_THEME_VARS);
  const nonce = randomToken();
  return (
    `<!DOCTYPE html><html${opts.hlOff ? ` class="hl-off"` : ''}><head>` +
    `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; ` +
    `style-src 'unsafe-inline'; img-src data:; script-src 'nonce-${nonce}'; ` +
    `form-action 'none'; base-uri 'none';">` +
    // Order matters only in one direction: the reset and the mark styling go
    // first so the variant scaffold (and, after it, the document's own rules)
    // override them on source order rather than needing extra specificity.
    `<style>:root{${vars}}` +
    OFFICE_BASE_CSS +
    officeVariantCss(variant, opts) +
    SANDBOX_SCROLLBAR_CSS +
    `</style></head><body><div id="${OFFICE_ZOOM_ID}">${html}</div>` +
    `<script nonce="${nonce}">${officeBootstrap(variant, opts)}</script>` +
    `</body></html>`
  );
}
