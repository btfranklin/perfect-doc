# README banner

This AI-generated cover shows a collection of linked documents and a validation
mark. The title uses the letter shapes of the Perfect Dark (BRK) font.

- File: [perfect_doc_social_preview.jpg](perfect_doc_social_preview.jpg).
- Dimensions: 1774 by 887 pixels.
- Generation tool: built-in `image_gen`, with a font specimen as the reference.
- Encoding: JPEG at quality 92. The image size was kept.
- Visible title: `Perfect Doc`.
- Title reference: `Perfect Doc` rendered in the installed Perfect Dark (BRK)
  font. The font file is not included in this repository.

## README integration

The main README places the image directly below its first heading:

```markdown
![Perfect Doc banner](https://raw.githubusercontent.com/btfranklin/perfect-doc/main/.github/social%20preview/perfect_doc_social_preview.jpg "Perfect Doc")
```

The public URL targets `btfranklin/perfect-doc` on the `main` branch. It becomes
available after the image is pushed there.

## Generation prompt

```text
Use case: ads-marketing.
Asset type: AI-generated GitHub repository social-preview banner and README cover. Wide landscape composition, approximately 2:1.
Transform the supplied font specimen into a complete promotional cover for Perfect Doc, a Rust tool that checks the structure of Markdown and HTML documentation collections.
Reference image: typography guide only. It shows the exact words Perfect Doc twice in the installed Perfect Dark (BRK) font. Use its angular letter shapes, long joined top strokes, cut corners, wide proportions, and deliberate gaps between strokes. Render one title only. Do not include the specimen's white background or duplicate title.
Text (verbatim): "Perfect Doc". This is the only text. Preserve the distinctive letter shapes closely. Large, clear, pale silver lettering, readable at thumbnail size.
Scene: a dark, cinematic technical workspace with a small collection of precise document sheets and translucent document panels. Their visible content consists only of abstract heading bars, indentation, angle-bracket symbols, and link nodes. Thin luminous paths connect headings, anchors, and documents. A restrained green verification mark suggests that the document structure and links have passed validation.
Style: polished science-fiction technical cover with a subtle late-1990s espionage-game mood. Deep navy and near-black background, cold blue edge light, silver lettering, small green verification accents. Crisp forms, restrained detail, and quiet depth.
Composition: the title spans the upper middle of the cover with clear space around it. The linked document collection occupies the lower half. Keep the title and main illustration well inside the edges. The typography is the main feature; the document forms support it.
Avoid extra words, taglines, badges, watermarks, copied game artwork, characters, weapons, robots, and dense unreadable interface text. This is promotional artwork, not a screenshot of the tool.
```
