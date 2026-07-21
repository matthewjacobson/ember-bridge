/**
 * In-app user manual. The content lives in `/docs/*.md` (the single source of
 * truth, also viewable on GitHub); here we import those files raw and render
 * them with the lightweight `Markdown` component. Add a chapter by dropping a
 * new file in `/docs` and listing it below.
 */

import { useState } from "react";
import { Markdown } from "../components/Markdown";
import gettingStarted from "../../docs/getting-started.md?raw";
import connectingEmber from "../../docs/connecting-ember.md?raw";
import addingMachines from "../../docs/adding-machines.md?raw";
import dongleSetup from "../../docs/dongle-setup.md?raw";
import sendingDesigns from "../../docs/sending-designs.md?raw";
import troubleshooting from "../../docs/troubleshooting.md?raw";

const CHAPTERS = [
  { id: "getting-started", title: "Getting started", body: gettingStarted },
  { id: "connecting-ember", title: "Connecting Ember", body: connectingEmber },
  { id: "adding-machines", title: "Adding machines", body: addingMachines },
  { id: "dongle-setup", title: "Setting up a dongle", body: dongleSetup },
  { id: "sending-designs", title: "Sending a design", body: sendingDesigns },
  { id: "troubleshooting", title: "Troubleshooting", body: troubleshooting },
] as const;

export function HelpPage() {
  const [activeId, setActiveId] = useState<string>(CHAPTERS[0].id);
  const chapter = CHAPTERS.find((c) => c.id === activeId) ?? CHAPTERS[0];

  return (
    <div className="help-page">
      <nav className="help-toc">
        {CHAPTERS.map((c) => (
          <button
            key={c.id}
            className={`help-toc-item ${c.id === activeId ? "active" : ""}`}
            onClick={() => setActiveId(c.id)}
          >
            {c.title}
          </button>
        ))}
      </nav>
      <article className="help-content">
        <Markdown source={chapter.body} />
      </article>
    </div>
  );
}
