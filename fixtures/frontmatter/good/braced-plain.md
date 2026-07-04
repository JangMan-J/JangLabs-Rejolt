---
name: config-expander
description: expands {HOME} at runtime
metadata:
  tags: [config]
  triggers:
    args:
      - a{b}
    paths:
      - /foo#bar
    synonyms: [c#, f#]
---
body
