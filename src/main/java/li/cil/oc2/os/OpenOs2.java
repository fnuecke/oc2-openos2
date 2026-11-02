package li.cil.oc2.os;

import java.io.InputStream;

public final class OpenOs2 {
    public static InputStream getDiskImage() {
        return open("generated/oc-openos2/disk.img");
    }

    private OpenOs2() {
    }

    private static InputStream open(final String resource) {
        final InputStream stream = OpenOs2.class.getClassLoader().getResourceAsStream(resource);
        if (stream == null) {
            throw new IllegalStateException("Missing resource [" + resource + "].");
        }
        return stream;
    }
}
