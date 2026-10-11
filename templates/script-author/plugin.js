/// <reference types="@nora/plugin-kit" />

nora.registerPanel('com.example.nora.hello.panel', event => nora.ui.column({
  children: [
    nora.ui.text('Hello Nora'),
    nora.ui.textField({ id: 'message', placeholder: 'Your message', value: event.values.message ?? '' }),
    nora.ui.button({ text: 'Echo', action: 'echo' }),
    nora.ui.text(event.action === 'echo' ? event.values.message ?? '' : 'A native interface powered by JavaScript.'),
  ],
}))

nora.registerCommand('com.example.nora.hello.echo', arguments_ => ({ text: arguments_.text }))
