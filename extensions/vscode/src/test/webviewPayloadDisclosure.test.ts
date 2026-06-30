import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { PayloadDisclosure, type PayloadDisclosureProps } from "../webview/PayloadDisclosure";

const payloadDisclosureSource = fs.readFileSync(path.join(process.cwd(), "src", "webview", "PayloadDisclosure.tsx"), "utf8");

function renderPayloadDisclosure(props: Partial<PayloadDisclosureProps> = {}): string {
  return renderToStaticMarkup(
    React.createElement(PayloadDisclosure, {
      props: {
        id: "payload-1",
        className: "raw",
        label: "Details",
        bodyHtml: '<pre class="code-frame">{}</pre>',
        ...props
      }
    })
  );
}

test("renders payload disclosures through a controlled accordion state", () => {
  const closed = renderPayloadDisclosure({ defaultOpen: false });
  const open = renderPayloadDisclosure({ defaultOpen: true });

  assert.match(closed, /data-slot="accordion"/);
  assert.match(closed, /aria-expanded="false"/);
  assert.match(open, /aria-expanded="true"/);
  assert.match(open, /code-frame/);
  assert.match(payloadDisclosureSource, /import \{ useSyncedAccordionValue \} from "\.\/syncedAccordionState"/);
  assert.match(payloadDisclosureSource, /useSyncedAccordionValue\(payloadDisclosureOpenValue\(props\)\)/);
  assert.match(payloadDisclosureSource, /value=\{openValue\}[\s\S]*onValueChange=\{setOpenValue\}/);
  assert.doesNotMatch(payloadDisclosureSource, /defaultValue=\{props\.defaultOpen/);
});
