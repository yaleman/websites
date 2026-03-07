import { Editor } from "@tiptap/core";
import Link from "@tiptap/extension-link";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";
import "./editor.css";

const bindToolbar = (editor: Editor) => {
  const toolbar = document.querySelector<HTMLElement>("[data-editor-toolbar]");
  if (!toolbar) {
    return;
  }

  const handleCommand = (commandName: string) => {
    switch (commandName) {
      case "bold":
        editor.chain().focus().toggleBold().run();
        return;
      case "italic":
        editor.chain().focus().toggleItalic().run();
        return;
      case "code":
        editor.chain().focus().toggleCode().run();
        return;
      case "link": {
        const href = window.prompt("Link URL");
        if (!href) {
          editor.chain().focus().unsetLink().run();
          return;
        }
        editor.chain().focus().setLink({ href }).run();
        return;
      }
      case "h2":
        editor.chain().focus().toggleHeading({ level: 2 }).run();
        return;
      case "h3":
        editor.chain().focus().toggleHeading({ level: 3 }).run();
        return;
      case "ul":
        editor.chain().focus().toggleBulletList().run();
        return;
      case "ol":
        editor.chain().focus().toggleOrderedList().run();
        return;
      case "quote":
        editor.chain().focus().toggleBlockquote().run();
        return;
      default:
        return;
    }
  };

  toolbar.addEventListener("click", (event) => {
    const target = event.target as HTMLElement | null;
    if (!target) {
      return;
    }

    const button = target.closest<HTMLButtonElement>("button[data-command]");
    if (!button) {
      return;
    }

    const commandName = button.getAttribute("data-command");
    if (!commandName) {
      return;
    }

    event.preventDefault();
    handleCommand(commandName);
  });
};

const initEditor = () => {
  const root = document.getElementById("editor");
  const textarea = document.getElementById("page_content") as HTMLTextAreaElement | null;

  if (!root || !textarea) {
    return;
  }

  textarea.style.display = "none";

  try {
    const editor = new Editor({
      element: root,
      content: textarea.value || "",
      contentType: "markdown",
      extensions: [
        StarterKit,
        Link.configure({ openOnClick: false }),
        Markdown,
      ],
      onUpdate: ({ editor }) => {
        textarea.value = editor.getMarkdown();
      },
    });

    bindToolbar(editor);
    document.body?.classList.add("editor-ready");
  } catch {
    textarea.style.display = "";
    document.body?.classList.add("editor-error");
  }
};

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initEditor);
} else {
  initEditor();
}
