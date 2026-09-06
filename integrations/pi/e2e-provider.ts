/**
 * A scripted provider for the end-to-end test: no API key, no network, no model.
 *
 * What an E2E of this extension has to prove is that PI dispatches a tool call into the execute()
 * registered here and hands the result back. A real model is the part that PRODUCES the call, and
 * that half tests the model rather than this code, so it is the one part replaced. Everything
 * between the call and the result is pi's own machinery and a real kern box.
 *
 * `KERN_E2E_SCRIPT` is a JSON array of `{name, arguments}`, one tool call per turn; when it runs out
 * the provider stops the turn with text, which is how pi knows to finish.
 */
import { createAssistantMessageEventStream } from "@earendil-works/pi-ai";

const SCRIPT: Array<{ name: string; arguments: Record<string, unknown> }> = JSON.parse(
	process.env.KERN_E2E_SCRIPT ?? "[]",
);
let turn = 0;

function scripted(model: any, _context: unknown, _options?: unknown): any {
	const stream = createAssistantMessageEventStream();
	(async () => {
		const msg: any = {
			role: "assistant",
			content: [],
			api: model.api,
			provider: model.provider,
			model: model.id,
			usage: {
				input: 0,
				output: 0,
				cacheRead: 0,
				cacheWrite: 0,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			stopReason: "pending",
		};
		stream.push({ type: "start", partial: msg });
		const step = SCRIPT[turn];
		turn += 1;
		if (step) {
			const call: any = { type: "toolCall", id: `kern-e2e-${turn}`, name: step.name, arguments: step.arguments };
			msg.content.push(call);
			stream.push({ type: "toolcall_start", contentIndex: 0, partial: msg });
			stream.push({ type: "toolcall_end", contentIndex: 0, toolCall: call, partial: msg });
			msg.stopReason = "toolUse";
			stream.push({ type: "done", reason: "toolUse", message: msg });
		} else {
			msg.content.push({ type: "text", text: "E2E-DONE" });
			stream.push({ type: "text_start", contentIndex: 0, partial: msg });
			stream.push({ type: "text_end", contentIndex: 0, content: "E2E-DONE", partial: msg });
			msg.stopReason = "stop";
			stream.push({ type: "done", reason: "stop", message: msg });
		}
	})();
	return stream;
}

export default function (pi: any): void {
	pi.registerProvider("kern-e2e", {
		baseUrl: "http://127.0.0.1",
		apiKey: "unused",
		api: "kern-e2e-api",
		models: [
			{
				id: "scripted",
				name: "scripted (no network)",
				reasoning: false,
				input: ["text"],
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
				contextWindow: 8192,
				maxTokens: 1024,
			},
		],
		streamSimple: scripted,
	});
}
