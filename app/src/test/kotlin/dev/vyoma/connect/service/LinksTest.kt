package dev.vyoma.connect.service

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LinksTest {
    @Test fun onlyWebLinksCount() {
        for (fine in listOf("https://example.com/a?b=c", "http://example.com", " HTTPS://example.com ")) {
            assertTrue(fine, LinkService.looksLikeUrl(fine))
        }
        for (refused in listOf("tel:123", "intent://x#Intent;end", "content://x/y", "file:///etc/passwd", "market://details?id=x", "javascript:alert(1)", "https://a b", "plain text")) {
            assertFalse(refused, LinkService.looksLikeUrl(refused))
        }
    }
}
