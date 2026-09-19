import io
import sys
import unittest
import urllib.error
from contextlib import redirect_stderr
from unittest import mock

import claude_limits_png


class FakeImage:
    def save(self, target, format):
        self.assert_format(format)
        target.write(b"png bytes")

    @staticmethod
    def assert_format(format):
        if format != "PNG":
            raise AssertionError(f"unexpected image format: {format}")


class PushRefusalDiagnosticsTest(unittest.TestCase):
    def run_refusal(self, body):
        error = urllib.error.HTTPError(
            "https://example.test/push",
            422,
            "Unprocessable Entity",
            {},
            io.BytesIO(body),
        )
        stderr = io.StringIO()
        argv = [
            "claude_limits_png.py",
            "--feed",
            "https://example.test/feed",
            "--push",
            "https://example.test/push",
        ]

        with (
            mock.patch.object(sys, "argv", argv),
            mock.patch.object(
                claude_limits_png, "fetch", return_value={}
            ) as fetch,
            mock.patch.object(
                claude_limits_png, "draw_panel", return_value=FakeImage()
            ) as draw_panel,
            mock.patch.object(
                claude_limits_png.urllib.request, "urlopen", side_effect=error
            ) as urlopen,
            redirect_stderr(stderr),
        ):
            result = claude_limits_png.main()

        fetch.assert_called_once_with("https://example.test/feed")
        draw_panel.assert_called_once_with({})
        request = urlopen.call_args.args[0]
        self.assertEqual(request.full_url, "https://example.test/push")
        self.assertEqual(request.get_method(), "POST")
        self.assertEqual(request.data, b"png bytes")
        self.assertEqual(request.headers["Content-type"], "image/png")
        self.assertEqual(
            request.headers["User-agent"],
            claude_limits_png.PRODUCER_USER_AGENT,
        )
        self.assertEqual(urlopen.call_args.kwargs, {"timeout": 30})
        return result, stderr.getvalue()

    def test_valid_utf8_refusal_is_reported(self):
        result, stderr = self.run_refusal("not allowed: café".encode())

        self.assertEqual(result, 1)
        self.assertEqual(stderr, "push refused 422: not allowed: café\n")

    def test_malformed_utf8_refusal_is_reported_with_replacement_character(self):
        result, stderr = self.run_refusal(b"not allowed: \xff")

        self.assertEqual(result, 1)
        self.assertEqual(stderr, "push refused 422: not allowed: \ufffd\n")


if __name__ == "__main__":
    unittest.main()
