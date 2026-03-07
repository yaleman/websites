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
    .config(nord)
    .config((editorCtx) => {
      editorCtx.set(rootCtx, root);
      editorCtx.set(defaultValueCtx, textarea.value || "");
      editorCtx.get(listenerCtx).markdownUpdated((_ctx, markdown) => {
        textarea.value = markdown;
      });
    })
    .use(commonmark)
    .use(history)
    .use(listener)
    .create();

  textarea.style.display = "none";
}
