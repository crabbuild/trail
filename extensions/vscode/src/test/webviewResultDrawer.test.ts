import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const source = fs.readFileSync(path.join(process.cwd(), "src", "webview", "ResultDrawer.tsx"), "utf8");

test("result drawer composes shadcn drawer button and badge primitives", () => {
  assert.match(
    source,
    /import \{[\s\S]*Accordion,[\s\S]*AccordionContent,[\s\S]*AccordionItem,[\s\S]*AccordionTrigger[\s\S]*\} from "@\/webview\/components\/ui\/accordion"/
  );
  assert.match(source, /import \{ Badge \} from "@\/webview\/components\/ui\/badge"/);
  assert.match(source, /import \{ Button \} from "@\/webview\/components\/ui\/button"/);
  assert.match(
    source,
    /import \{[\s\S]*Drawer,[\s\S]*DrawerContent,[\s\S]*DrawerDescription,[\s\S]*DrawerHeader,[\s\S]*DrawerTitle[\s\S]*\} from "@\/webview\/components\/ui\/drawer"/
  );
  assert.match(source, /<Drawer[\s\S]*direction="right"[\s\S]*open/);
  assert.match(source, /<DrawerContent className=\{cn\("json-drawer result-drawer", props\.className\)\}/);
  assert.match(source, /<Badge className="result-drawer-badge" variant="outline">/);
  assert.match(source, /<Button[\s\S]*data-action="closeDrawer"[\s\S]*size="icon-sm"[\s\S]*variant="ghost"/);
  assert.doesNotMatch(source, /className="icon-button"/);
  assert.match(source, /<XIcon aria-hidden="true" data-icon="inline-start" \/>/);
});

test("result drawer preserves helper-rendered body and lifecycle selectors", () => {
  assert.match(source, /className="result-drawer-body"/);
  assert.match(source, /dangerouslySetInnerHTML=\{\{ __html: props\.bodyHtml \}\}/);
  assert.match(source, /element\.dataset\.resultDrawerHost = ""/);
  assert.match(source, /querySelector<HTMLElement>\("\[data-action='closeDrawer'\]"\)\?\.focus\(\)/);
  assert.match(source, /document\.querySelector<HTMLElement>\("\.json-drawer"\)/);
});

test("result drawer mounts shadcn accordion widgets into helper placeholders", () => {
  assert.match(source, /import \{ createPortal \} from "react-dom"/);
  assert.match(source, /widgets\?: ResultDrawerWidget\[\] \| undefined/);
  assert.match(source, /querySelectorAll<HTMLElement>\("\[data-result-drawer-widget\]"\)/);
  assert.match(source, /createPortal\(<ResultDrawerWidgetView widget=\{widget\} \/>/);
  assert.match(source, /<Accordion className=\{widget\.className\} defaultValue=\{defaultValue\} multiple=\{widget\.multiple\}>/);
  assert.match(source, /<AccordionTrigger className=\{item\.triggerClassName\}>/);
  assert.match(source, /<AccordionContent className=\{item\.contentClassName\} keepMounted>/);
});
