// The Una website (#244): highlighting for `una` code blocks, and the two Una themes in the
// theme menu. mdBook highlights every block before this script runs, so `una` blocks are
// highlighted again here, with the keywords of `spec/01 §5` and the edition 0.2 ones.
(function () {
    "use strict";

    if (typeof hljs !== "undefined") {
        hljs.registerLanguage("una", function (hljs) {
            return {
                name: "Una",
                keywords: {
                    keyword:
                        "module import pub kind capability op handler policy predicate requirement " +
                        "example trusted reflect opaque fn let edit take mod where in uses fails " +
                        "requires ensures derived from accepts given do expect performed trait impl",
                    built_in:
                        "if then else when is for together with attempt checked fail pass use " +
                        "error none implies and or not it result old",
                    meta: "public internal confidential secret restricted",
                    literal: "true false unit",
                    type:
                        "Int Nat Bool Text Unit List Option Result Hashed Map Entry Json " +
                        "U8 U16 U32 U64 I8 I16 I32 I64",
                },
                contains: [
                    { className: "doctag", begin: /---/, end: /$/ },
                    hljs.COMMENT(/--/, /$/),
                    {
                        className: "string",
                        begin: /"/,
                        end: /"/,
                        contains: [hljs.BACKSLASH_ESCAPE, { className: "subst", begin: /\{/, end: /\}/ }],
                    },
                    { className: "number", begin: /\b\d[\d_]*\b/ },
                    { className: "variable", begin: /@[a-z_][a-zA-Z0-9_]*/ },
                    { className: "type", begin: /\b[A-Z][a-zA-Z0-9]*\b/ },
                    { className: "title", begin: /(?<=^(pub )?)[a-z_][a-zA-Z0-9_]*(?=(\[[^\]]*\])?\()/ },
                ],
            };
        });
        document.querySelectorAll("code.language-una").forEach(function (block) {
            block.textContent = block.textContent;
            hljs.highlightBlock(block);
        });
    }

    // The theme menu offers Auto, Una light and Una dark.
    var names = { light: "Una light", navy: "Una dark" };
    ["rust", "coal", "ayu"].forEach(function (id) {
        var button = document.getElementById("mdbook-theme-" + id);
        if (button && button.parentElement) {
            button.parentElement.style.display = "none";
        }
    });
    Object.keys(names).forEach(function (id) {
        var button = document.getElementById("mdbook-theme-" + id);
        if (button) {
            button.textContent = names[id];
        }
    });
})();
