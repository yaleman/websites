import { Editor, defaultValueCtx, rootCtx } from "@milkdown/core";
import { commonmark } from "@milkdown/preset-commonmark";
import { nord } from "@milkdown/theme-nord";
import { history } from "@milkdown/plugin-history";
import { listener, listenerCtx } from "@milkdown/plugin-listener";
import "./editor.css";

const root = document.getElementById("editor");
const textarea = document.getElementById("page_content") as HTMLTextAreaElement | null;

if (root && textarea) {
  Editor.make()
    .config((ctx) => {
      ctx.set(rootCtx, root);
      ctx.set(defaultValueCtx, textarea.value || "");
      ctx.get(listenerCtx).markdownUpdated((_, markdown) => {
        textarea.value = markdown;
      });
    })
    .use(nord)
    .use(commonmark)
    .use(history)
    .use(listener)
    .create();

  textarea.style.display = "none";
}
